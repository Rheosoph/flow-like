use flow_like_storage::{
    Path as ObjectPath, decode_path_segment,
    files::store::FlowLikeStore,
    object_store::{
        GetOptions, ObjectMeta, ObjectStore, ObjectStoreExt, PutMode, PutOptions, UpdateVersion,
    },
};
use flow_like_types::{Context, Result, anyhow, bail, futures::StreamExt, json, tokio};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

const LOCK: &str = ".git/flow-like-store.lock";
const METADATA: &str = ".git/flow-like-worktree.json";

#[derive(Clone, Default, Serialize, Deserialize)]
struct FileMetadata {
    executable: bool,
    symlink: bool,
}

struct StoredFile {
    meta: ObjectMeta,
    hash: flow_like_storage::blake3::Hash,
}

enum FileContents {
    Disk(PathBuf),
    Inline(Vec<u8>),
}

struct WorkspaceFile {
    contents: FileContents,
    hash: flow_like_storage::blake3::Hash,
}

impl WorkspaceFile {
    fn disk(path: PathBuf) -> Result<Self> {
        let mut hash = flow_like_storage::blake3::Hasher::new();
        hash.update_reader(std::fs::File::open(&path)?)?;
        Ok(Self {
            contents: FileContents::Disk(path),
            hash: hash.finalize(),
        })
    }

    fn inline(bytes: Vec<u8>) -> Self {
        Self {
            hash: flow_like_storage::blake3::hash(&bytes),
            contents: FileContents::Inline(bytes),
        }
    }

    async fn bytes(&self) -> Result<Vec<u8>> {
        match &self.contents {
            FileContents::Disk(path) => Ok(tokio::fs::read(path).await?),
            FileContents::Inline(bytes) => Ok(bytes.clone()),
        }
    }
}

struct Remote {
    store: Arc<dyn ObjectStore>,
    prefix: ObjectPath,
    original: BTreeMap<String, StoredFile>,
    lock: ObjectPath,
    released: AtomicBool,
    saving: AtomicBool,
    attributes: BTreeMap<String, FileMetadata>,
}

pub(crate) struct Workspace {
    path: PathBuf,
    temporary: Option<PathBuf>,
    remote: Option<Remote>,
}

impl Workspace {
    #[cfg(feature = "execute")]
    pub(crate) async fn open(
        context: &mut flow_like::flow::execution::context::ExecutionContext,
        path: &crate::data::path::FlowPath,
    ) -> Result<Self> {
        Self::from_store(path.to_store(context).await?, path.object_path()).await
    }

    pub(crate) async fn from_store(store: FlowLikeStore, prefix: ObjectPath) -> Result<Self> {
        if let FlowLikeStore::Local(local) = store {
            return Ok(Self {
                path: super::resolve_local_path(&local, &prefix)?,
                temporary: None,
                remote: None,
            });
        }
        let store = store.as_generic();
        let lock = object_key(&prefix, LOCK);
        store.put_opts(
            &lock,
            uuid::Uuid::new_v4().to_string().into(),
            PutOptions { mode: PutMode::Create, ..Default::default() },
        ).await.map_err(|error| anyhow!(
            "Could not lock repository storage at {lock}: {error}. Another Git operation may be using it. If its runtime stopped, remove the stale lock before retrying."
        ))?;
        let temporary =
            std::env::temp_dir().join(format!("flow-like-git-{}", uuid::Uuid::new_v4()));
        let mut workspace = Self {
            path: temporary.join("repository"),
            temporary: Some(temporary.clone()),
            remote: Some(Remote {
                store,
                prefix,
                original: BTreeMap::new(),
                lock,
                released: AtomicBool::new(false),
                saving: AtomicBool::new(false),
                attributes: BTreeMap::new(),
            }),
        };
        let result = async {
            tokio::fs::create_dir(&temporary).await?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                tokio::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o700))
                    .await?;
            }
            workspace.download().await
        }
        .await;
        if let Err(error) = result {
            workspace.release().await?;
            return Err(error);
        }
        Ok(workspace)
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    async fn download(&mut self) -> Result<()> {
        let remote = self.remote.as_mut().unwrap();
        let listed = list_files(remote).await?;
        let mut metadata = BTreeMap::<String, FileMetadata>::new();
        let mut directories = BTreeSet::new();
        for (relative, meta) in listed {
            let file = remote
                .store
                .get_opts(
                    &meta.location,
                    GetOptions {
                        if_match: meta.e_tag.clone(),
                        version: meta.version.clone(),
                        ..Default::default()
                    },
                )
                .await?;
            let meta = file.meta.clone();
            let bytes = file.bytes().await?;
            if relative == METADATA {
                metadata = json::from_slice(&bytes).context("Invalid stored Git file metadata")?;
            } else {
                let path = safe_path(&self.path, &relative)?;
                create_download_directory(&self.path, path.parent().unwrap(), &mut directories)
                    .await?;
                use tokio::io::AsyncWriteExt;
                let mut output = tokio::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(path)
                    .await
                    .context("Stored repository filenames collide on this filesystem")?;
                output.write_all(&bytes).await?;
                output.flush().await?;
            }
            remote.original.insert(
                relative,
                StoredFile {
                    meta,
                    hash: flow_like_storage::blake3::hash(&bytes),
                },
            );
        }
        // Make links last so no downloaded object can be written through a link.
        for (relative, attributes) in &metadata {
            let path = safe_path(&self.path, &relative)?;
            if is_git_path(relative) {
                bail!("Stored file metadata must not redirect Git metadata");
            }
            if !remote.original.contains_key(relative) {
                continue;
            }
            if attributes.symlink {
                let target = tokio::fs::read_to_string(&path).await?;
                tokio::fs::remove_file(&path).await?;
                create_symlink(Path::new(&target), &path)?;
            } else if attributes.executable {
                set_executable(&path)?;
            }
        }
        remote.attributes = metadata;
        validate_git_directory(&self.path)?;
        if self.path.join(".git").is_dir() {
            // Object stores do not retain the empty directories Git requires.
            for relative in [".git/objects", ".git/refs"] {
                create_download_directory(&self.path, &self.path.join(relative), &mut directories)
                    .await?;
            }
        }
        Ok(())
    }

    pub(crate) async fn persist(&self) -> Result<()> {
        self.persist_filtered(true).await
    }

    #[cfg(feature = "execute")]
    pub(crate) async fn discard(&self) -> Result<()> {
        self.release().await
    }

    pub(crate) async fn persist_filtered(&self, include_git: bool) -> Result<()> {
        let result = self.save(include_git).await;
        if let Err(error) = &result
            && let Some(remote) = &self.remote
            && remote.saving.load(Ordering::SeqCst)
        {
            return Err(anyhow!(
                "{error}. Repository storage may be partially updated. The recovery checkout is retained at {} and the storage lock at {}. Restore the checkout to storage before removing the lock.",
                self.path.display(),
                remote.lock
            ));
        }
        let release = self.release().await;
        match (result, release) {
            (Err(error), Err(release)) => Err(anyhow!(
                "{error}; could not release storage lock: {release}"
            )),
            (Err(error), _) | (_, Err(error)) => Err(error),
            _ => Ok(()),
        }
    }

    pub(crate) async fn finish<T>(&self, result: Result<T>) -> Result<T> {
        // Failed merges and stash applications can leave useful conflict state.
        match (result, self.persist().await) {
            (Err(error), Err(save)) => Err(anyhow!(
                "{error}; could not save repository changes: {save}"
            )),
            (_, Err(save)) => Err(anyhow!("Could not save repository changes: {save}")),
            (result, Ok(())) => result,
        }
    }

    async fn save(&self, include_git: bool) -> Result<()> {
        let Some(remote) = &self.remote else {
            return Ok(());
        };
        if remote.released.load(Ordering::SeqCst) {
            bail!("Repository workspace is already closed");
        }
        let path = self.path.clone();
        let attributes = remote.attributes.clone();
        let files =
            tokio::task::spawn_blocking(move || collect_files(&path, include_git, &attributes))
                .await??;
        let current = list_files(remote).await?;
        if current.len() != remote.original.len()
            || current.iter().any(|(relative, meta)| {
                remote
                    .original
                    .get(relative)
                    .is_none_or(|original| !same_version(meta, &original.meta))
            })
        {
            bail!(
                "Repository storage changed during the Git operation. Changes were not saved; retry using the current stored repository."
            );
        }
        // Save objects before refs, so published references have their objects available.
        let mut changed: Vec<_> = files
            .iter()
            .filter(|(relative, file)| {
                remote
                    .original
                    .get(*relative)
                    .is_none_or(|original| original.hash != file.hash)
            })
            .collect();
        changed.sort_by_key(|(name, _)| {
            (
                name.starts_with(".git/refs/") || name.as_str() == ".git/HEAD",
                *name,
            )
        });
        for (relative, file) in changed {
            let location = remote
                .original
                .get(relative)
                .map(|original| original.meta.location.clone())
                .unwrap_or_else(|| object_key(&remote.prefix, relative));
            let mode = match remote.original.get(relative) {
                Some(original) => PutMode::Update(UpdateVersion {
                    e_tag: original.meta.e_tag.clone(),
                    version: original.meta.version.clone(),
                }),
                None => PutMode::Create,
            };
            let bytes = file.bytes().await?;
            remote.saving.store(true, Ordering::SeqCst);
            remote
                .store
                .put_opts(
                    &location,
                    bytes.into(),
                    PutOptions {
                        mode,
                        ..Default::default()
                    },
                )
                .await
                .with_context(|| format!("Failed to save repository file {relative}"))?;
        }
        for (relative, original) in &remote.original {
            if !files.contains_key(relative) {
                // The storage API has no conditional delete. Recheck just before deleting.
                let current = remote.store.head(&original.meta.location).await?;
                if !same_version(&current, &original.meta) {
                    bail!(
                        "Repository file {relative} changed before deletion; retry the operation"
                    );
                }
                remote.saving.store(true, Ordering::SeqCst);
                remote.store.delete(&original.meta.location).await?;
            }
        }
        remote.saving.store(false, Ordering::SeqCst);
        Ok(())
    }

    async fn release(&self) -> Result<()> {
        if let Some(remote) = &self.remote
            && !remote.released.load(Ordering::SeqCst)
        {
            remote.store.delete(&remote.lock).await?;
            remote.released.store(true, Ordering::SeqCst);
        }
        Ok(())
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        if self
            .remote
            .as_ref()
            .is_some_and(|remote| remote.saving.load(Ordering::SeqCst))
        {
            return;
        }
        if let Some(remote) = &self.remote
            && !remote.released.load(Ordering::SeqCst)
            && let Ok(runtime) = tokio::runtime::Handle::try_current()
        {
            let store = remote.store.clone();
            let lock = remote.lock.clone();
            runtime.spawn(async move {
                let _ = store.delete(&lock).await;
            });
        }
        if let Some(temporary) = &self.temporary {
            let _ = std::fs::remove_dir_all(temporary);
        }
    }
}

fn object_key(prefix: &ObjectPath, relative: &str) -> ObjectPath {
    relative
        .split('/')
        .fold(prefix.clone(), |path, part| path.join(part))
}

async fn create_download_directory(
    root: &Path,
    directory: &Path,
    created: &mut BTreeSet<PathBuf>,
) -> Result<()> {
    let relative = directory.strip_prefix(root)?;
    let mut current = root.to_path_buf();
    let mut components = relative.components();
    loop {
        if !created.contains(&current) {
            // An existing uncached spelling aliases another downloaded path.
            match tokio::fs::create_dir(&current).await {
                Ok(()) => {
                    created.insert(current.clone());
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    bail!(
                        "Stored repository directory names collide on this filesystem: {}",
                        current.strip_prefix(root)?.display()
                    );
                }
                Err(error) => return Err(error.into()),
            }
        }
        match components.next() {
            Some(component) => current.push(component),
            None => break,
        }
    }
    Ok(())
}

async fn list_files(remote: &Remote) -> Result<BTreeMap<String, ObjectMeta>> {
    let mut stream = remote.store.list(Some(&remote.prefix));
    let mut files = BTreeMap::new();
    while let Some(meta) = stream.next().await {
        let meta = meta?;
        if meta.location == remote.lock {
            continue;
        }
        let Some(parts) = meta.location.prefix_match(&remote.prefix) else {
            continue;
        };
        let relative = parts
            .map(|part| decode_path_segment(part.as_ref()).into_owned())
            .collect::<Vec<_>>();
        if relative.is_empty() {
            bail!("Repository path points to a file");
        }
        for part in &relative {
            if part.is_empty()
                || part == "."
                || part == ".."
                || part.contains(['/', '\0'])
                || (cfg!(windows) && part.contains(['\\', ':']))
            {
                bail!("Stored repository contains an unsafe path component: {part:?}");
            }
        }
        if relative
            .first()
            .is_some_and(|part| part.eq_ignore_ascii_case(".git") && part != ".git")
        {
            bail!("Stored Git metadata must use the directory name .git");
        }
        if files.insert(relative.join("/"), meta).is_some() {
            bail!("Stored repository contains colliding file paths");
        }
    }
    Ok(files)
}

fn safe_path(root: &Path, relative: &str) -> Result<PathBuf> {
    if relative.is_empty()
        || relative.contains('\0')
        || (cfg!(windows) && relative.contains(['\\', ':']))
        || relative
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        || Path::new(relative)
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        bail!("Stored repository contains an unsafe path: {relative:?}");
    }
    let path = root.join(relative);
    let mut parent = path.parent();
    while let Some(ancestor) = parent {
        if ancestor == root {
            break;
        }
        if std::fs::symlink_metadata(ancestor)
            .is_ok_and(|metadata| metadata.file_type().is_symlink())
        {
            bail!("Repository path passes through a symlink: {relative}");
        }
        parent = ancestor.parent();
    }
    Ok(path)
}

fn same_version(left: &ObjectMeta, right: &ObjectMeta) -> bool {
    if left.size != right.size {
        return false;
    }
    // Cloud listings can omit version IDs or round timestamps that GET returns.
    let etag = left.e_tag.as_ref().zip(right.e_tag.as_ref());
    let version = left.version.as_ref().zip(right.version.as_ref());
    if etag.is_some_and(|(left, right)| left != right)
        || version.is_some_and(|(left, right)| left != right)
    {
        return false;
    }
    etag.is_some() || version.is_some() || left.last_modified == right.last_modified
}

fn is_git_path(relative: &str) -> bool {
    relative
        .split('/')
        .next()
        .is_some_and(|part| part.eq_ignore_ascii_case(".git"))
}

fn validate_git_directory(root: &Path) -> Result<()> {
    let git = root.join(".git");
    if git.is_file()
        || std::fs::symlink_metadata(&git).is_ok_and(|meta| meta.file_type().is_symlink())
    {
        bail!(
            "Cloud and memory repositories need their own .git directory. Clone the repository into this store instead of copying a linked worktree."
        );
    }
    for relative in [
        "commondir",
        "objects/info/alternates",
        "objects/info/http-alternates",
    ] {
        if git.join(relative).exists() {
            bail!("Stored repository references Git metadata outside its checkout: {relative}");
        }
    }
    Ok(())
}

fn collect_files(
    root: &Path,
    include_git: bool,
    previous_attributes: &BTreeMap<String, FileMetadata>,
) -> Result<BTreeMap<String, WorkspaceFile>> {
    #[cfg(unix)]
    let _ = previous_attributes;
    let mut files = BTreeMap::new();
    if !root.exists() {
        return Ok(files);
    }
    validate_git_directory(root)?;
    let mut attributes = BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(directory)? {
            let entry = entry?;
            let path = entry.path();
            let relative = path
                .strip_prefix(root)?
                .to_str()
                .ok_or_else(|| anyhow!("Repository filenames must be UTF-8"))?
                .replace(std::path::MAIN_SEPARATOR, "/");
            if relative == LOCK || relative == METADATA || (!include_git && is_git_path(&relative))
            {
                continue;
            }
            safe_path(root, &relative)?;
            let kind = entry.file_type()?;
            if kind.is_dir() {
                pending.push(path);
            } else if kind.is_symlink() {
                if is_git_path(&relative) {
                    bail!("Git metadata must not contain symlinks");
                }
                let target = std::fs::read_link(&path)?;
                let target = target
                    .to_str()
                    .ok_or_else(|| anyhow!("Repository symlink targets must be UTF-8"))?;
                files.insert(
                    relative.clone(),
                    WorkspaceFile::inline(target.as_bytes().to_vec()),
                );
                attributes.insert(
                    relative,
                    FileMetadata {
                        executable: false,
                        symlink: true,
                    },
                );
            } else if kind.is_file() {
                files.insert(relative.clone(), WorkspaceFile::disk(path)?);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if !is_git_path(&relative)
                        && entry.metadata()?.permissions().mode() & 0o100 != 0
                    {
                        attributes.insert(
                            relative,
                            FileMetadata {
                                executable: true,
                                symlink: false,
                            },
                        );
                    }
                }
                #[cfg(not(unix))]
                if previous_attributes
                    .get(&relative)
                    .is_some_and(|metadata| metadata.executable)
                {
                    attributes.insert(
                        relative,
                        FileMetadata {
                            executable: true,
                            symlink: false,
                        },
                    );
                }
            } else {
                bail!("Repository contains an unsupported file type: {relative}");
            }
        }
    }
    if include_git && root.join(".git").is_dir() {
        files.insert(
            METADATA.into(),
            WorkspaceFile::inline(json::to_vec(&attributes)?),
        );
    }
    Ok(files)
}

#[cfg(unix)]
fn create_symlink(target: &Path, path: &Path) -> Result<()> {
    std::os::unix::fs::symlink(target, path)?;
    Ok(())
}

#[cfg(windows)]
fn create_symlink(target: &Path, path: &Path) -> Result<()> {
    std::os::windows::fs::symlink_file(target, path)?;
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn create_symlink(_target: &Path, _path: &Path) -> Result<()> {
    bail!("This runtime cannot restore repository symlinks")
}

fn set_executable(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(path)?.permissions();
        permissions.set_mode(permissions.mode() | 0o111);
        std::fs::set_permissions(path, permissions)?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

#[cfg(test)]
#[path = "repository_workspace_tests.rs"]
mod tests;
