use chrono::{DateTime, Utc};
use flow_like_types::{Bytes, async_trait, sync::Mutex};
use futures::{StreamExt, stream};
use object_store::path::Path;
use object_store::{
    Attributes, GetOptions, GetResult, GetResultPayload, ListResult, MultipartUpload, ObjectMeta,
    ObjectStore, PutMode, PutMultipartOptions, PutOptions, PutPayload, PutResult, Result,
};
use smb2::{ClientConfig, DirectoryEntry, ErrorKind, FileInfo, SmbClient, Tree};
use std::collections::HashMap;
use std::fmt::{Debug, Display};
use std::sync::Arc;
use std::time::Duration;

const STORE_NAME: &str = "SMB";

#[derive(Clone)]
pub struct SmbConfig {
    pub addr: String,
    pub share: String,
    pub username: String,
    pub password: String,
    pub domain: String,
    pub timeout: Duration,
    pub auto_reconnect: bool,
    pub compression: bool,
    pub dfs_enabled: bool,
}

impl SmbConfig {
    pub fn new(
        addr: impl Into<String>,
        share: impl Into<String>,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        Self {
            addr: addr.into(),
            share: share.into(),
            username: username.into(),
            password: password.into(),
            domain: String::new(),
            timeout: Duration::from_secs(5),
            auto_reconnect: false,
            compression: true,
            dfs_enabled: true,
        }
    }
}

impl Debug for SmbConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SmbConfig")
            .field("addr", &self.addr)
            .field("share", &self.share)
            .field("username", &self.username)
            .field("password", &"[REDACTED]")
            .field("domain", &self.domain)
            .field("timeout", &self.timeout)
            .field("auto_reconnect", &self.auto_reconnect)
            .field("compression", &self.compression)
            .field("dfs_enabled", &self.dfs_enabled)
            .finish()
    }
}

struct SmbSession {
    client: SmbClient,
    tree: Tree,
}

#[derive(Clone)]
pub struct SmbObjectStore {
    config: SmbConfig,
    session: Arc<Mutex<SmbSession>>,
}

impl Debug for SmbObjectStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SmbObjectStore")
            .field("addr", &self.config.addr)
            .field("share", &self.config.share)
            .field("username", &self.config.username)
            .field("domain", &self.config.domain)
            .finish_non_exhaustive()
    }
}

impl Display for SmbObjectStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SMB({}/{})", self.config.addr, self.config.share)
    }
}

impl SmbObjectStore {
    pub async fn connect(config: SmbConfig) -> Result<Self> {
        if config.addr.trim().is_empty() {
            return Err(generic_error("SMB address is required"));
        }
        if config.share.trim().is_empty() {
            return Err(generic_error("SMB share is required"));
        }

        let client_config = ClientConfig {
            addr: config.addr.clone(),
            timeout: config.timeout,
            username: config.username.clone(),
            password: config.password.clone(),
            domain: config.domain.clone(),
            auto_reconnect: config.auto_reconnect,
            compression: config.compression,
            dfs_enabled: config.dfs_enabled,
            dfs_target_overrides: HashMap::new(),
        };

        let mut client = SmbClient::connect(client_config)
            .await
            .map_err(|err| map_smb_error(err, format!("//{}", config.addr)))?;
        let tree = client
            .connect_share(&config.share)
            .await
            .map_err(|err| map_smb_error(err, config.share.clone()))?;

        Ok(Self {
            config,
            session: Arc::new(Mutex::new(SmbSession { client, tree })),
        })
    }

    async fn stat_info(&self, path: &str) -> Result<FileInfo> {
        let mut session = self.session.lock().await;
        let SmbSession { client, tree } = &mut *session;
        client
            .stat(tree, path)
            .await
            .map_err(|err| map_smb_error(err, path.to_string()))
    }

    async fn ensure_parent_directories(&self, path: &str) -> Result<()> {
        for parent in parent_paths(path) {
            let mut session = self.session.lock().await;
            let SmbSession { client, tree } = &mut *session;
            match client.create_directory(tree, &parent).await {
                Ok(_) => {}
                Err(err) if err.kind() == ErrorKind::AlreadyExists => {}
                Err(err) => return Err(map_smb_error(err, parent)),
            }
        }

        Ok(())
    }

    async fn list_recursive_from(&self, prefix: &str) -> Result<Vec<ObjectMeta>> {
        if !prefix.is_empty() {
            match self.stat_info(prefix).await {
                Ok(info) if !info.is_directory => return Ok(vec![meta_from_info(prefix, &info)]),
                Ok(_) => {}
                Err(object_store::Error::NotFound { .. }) => return Ok(Vec::new()),
                Err(err) => return Err(err),
            }
        }

        let mut objects = Vec::new();
        let mut dirs = vec![prefix.to_string()];

        while let Some(dir) = dirs.pop() {
            let entries = {
                let mut session = self.session.lock().await;
                let SmbSession { client, tree } = &mut *session;
                client
                    .list_directory(tree, &dir)
                    .await
                    .map_err(|err| map_smb_error(err, dir.clone()))?
            };

            for entry in entries
                .into_iter()
                .filter(|entry| is_real_entry(&entry.name))
            {
                let path = join_path(&dir, &entry.name);
                if entry.is_directory {
                    dirs.push(path);
                } else {
                    objects.push(meta_from_entry(&path, &entry));
                }
            }
        }

        Ok(objects)
    }
}

#[async_trait]
impl ObjectStore for SmbObjectStore {
    async fn put_opts(
        &self,
        location: &Path,
        payload: PutPayload,
        opts: PutOptions,
    ) -> Result<PutResult> {
        reject_attributes(&opts.attributes)?;
        let path = object_path(location);

        match &opts.mode {
            PutMode::Overwrite => {}
            PutMode::Create => match self.stat_info(&path).await {
                Ok(_) => {
                    return Err(object_store::Error::AlreadyExists {
                        path,
                        source: "SMB path already exists".into(),
                    });
                }
                Err(object_store::Error::NotFound { .. }) => {}
                Err(err) => return Err(err),
            },
            PutMode::Update(expected) => {
                let current = self.head(location).await?;
                if expected.e_tag.is_some() && current.e_tag != expected.e_tag {
                    return Err(object_store::Error::Precondition {
                        path,
                        source: "SMB object ETag did not match update precondition".into(),
                    });
                }
                if expected.version.is_some() && current.version != expected.version {
                    return Err(object_store::Error::Precondition {
                        path,
                        source: "SMB object version did not match update precondition".into(),
                    });
                }
            }
        }

        self.ensure_parent_directories(&path).await?;
        let bytes = payload_to_bytes(payload);
        {
            let mut session = self.session.lock().await;
            let SmbSession { client, tree } = &mut *session;
            client
                .write_file_pipelined(tree, &path, &bytes)
                .await
                .map_err(|err| map_smb_error(err, path.clone()))?;
        }

        let e_tag = self.head(location).await.ok().and_then(|meta| meta.e_tag);
        Ok(PutResult {
            e_tag,
            version: None,
        })
    }

    async fn put_multipart_opts(
        &self,
        location: &Path,
        opts: PutMultipartOptions,
    ) -> Result<Box<dyn MultipartUpload>> {
        reject_attributes(&opts.attributes)?;
        Ok(Box::new(SmbMultipartUpload {
            store: self.clone(),
            location: location.clone(),
            parts: Vec::new(),
        }))
    }

    async fn get_opts(&self, location: &Path, options: GetOptions) -> Result<GetResult> {
        if let Some(version) = &options.version {
            return Err(object_store::Error::NotSupported {
                source: format!("SMB object versions are not supported: {version}").into(),
            });
        }

        let path = object_path(location);
        let meta = self.head(location).await?;
        options.check_preconditions(&meta)?;

        let range = match &options.range {
            Some(range) => {
                range
                    .as_range(meta.size)
                    .map_err(|err| object_store::Error::Generic {
                        store: STORE_NAME,
                        source: Box::new(err),
                    })?
            }
            None => 0..meta.size,
        };

        if options.head {
            return Ok(GetResult {
                payload: GetResultPayload::Stream(stream::empty().boxed()),
                meta,
                range: 0..0,
                attributes: Attributes::default(),
            });
        }

        let data = {
            let mut session = self.session.lock().await;
            let SmbSession { client, tree } = &mut *session;
            client
                .read_file_pipelined(tree, &path)
                .await
                .map_err(|err| map_smb_error(err, path.clone()))?
        };

        let bytes = Bytes::from(data);
        let start = usize::try_from(range.start).map_err(|_| range_error(&path))?;
        let end = usize::try_from(range.end).map_err(|_| range_error(&path))?;
        if start > bytes.len() || end > bytes.len() || start > end {
            return Err(range_error(&path));
        }
        let bytes = bytes.slice(start..end);

        Ok(GetResult {
            payload: GetResultPayload::Stream(stream::once(async move { Ok(bytes) }).boxed()),
            meta,
            range,
            attributes: Attributes::default(),
        })
    }

    async fn head(&self, location: &Path) -> Result<ObjectMeta> {
        let path = object_path(location);
        let info = self.stat_info(&path).await?;
        if info.is_directory {
            return Err(object_store::Error::NotFound {
                path,
                source: "SMB path is a directory".into(),
            });
        }

        Ok(meta_from_info(location.as_ref(), &info))
    }

    async fn delete(&self, location: &Path) -> Result<()> {
        let path = object_path(location);
        let info = self.stat_info(&path).await?;
        let mut session = self.session.lock().await;
        let SmbSession { client, tree } = &mut *session;

        if info.is_directory {
            client
                .delete_directory(tree, &path)
                .await
                .map_err(|err| map_smb_error(err, path))
        } else {
            client
                .delete_file(tree, &path)
                .await
                .map_err(|err| map_smb_error(err, path))
        }
    }

    fn list(
        &self,
        prefix: Option<&Path>,
    ) -> futures::stream::BoxStream<'static, Result<ObjectMeta>> {
        let store = self.clone();
        let prefix = prefix.map(object_path).unwrap_or_default();
        stream::once(async move { store.list_recursive_from(&prefix).await })
            .flat_map(|result| match result {
                Ok(objects) => stream::iter(objects.into_iter().map(Ok)).boxed(),
                Err(err) => stream::iter(vec![Err(err)]).boxed(),
            })
            .boxed()
    }

    async fn list_with_delimiter(&self, prefix: Option<&Path>) -> Result<ListResult> {
        let prefix = prefix.map(object_path).unwrap_or_default();
        if !prefix.is_empty() {
            match self.stat_info(&prefix).await {
                Ok(info) if !info.is_directory => {
                    return Ok(ListResult {
                        common_prefixes: Vec::new(),
                        objects: vec![meta_from_info(&prefix, &info)],
                    });
                }
                Ok(_) => {}
                Err(object_store::Error::NotFound { .. }) => {
                    return Ok(ListResult {
                        common_prefixes: Vec::new(),
                        objects: Vec::new(),
                    });
                }
                Err(err) => return Err(err),
            }
        }

        let entries = {
            let mut session = self.session.lock().await;
            let SmbSession { client, tree } = &mut *session;
            client
                .list_directory(tree, &prefix)
                .await
                .map_err(|err| map_smb_error(err, prefix.clone()))?
        };

        let mut common_prefixes = Vec::new();
        let mut objects = Vec::new();

        for entry in entries
            .into_iter()
            .filter(|entry| is_real_entry(&entry.name))
        {
            let path = join_path(&prefix, &entry.name);
            if entry.is_directory {
                common_prefixes.push(Path::from(path));
            } else {
                objects.push(meta_from_entry(&path, &entry));
            }
        }

        Ok(ListResult {
            common_prefixes,
            objects,
        })
    }

    async fn copy(&self, from: &Path, to: &Path) -> Result<()> {
        let bytes = self.get(from).await?.bytes().await?;
        self.put(to, PutPayload::from_bytes(bytes)).await?;
        Ok(())
    }

    async fn rename(&self, from: &Path, to: &Path) -> Result<()> {
        let from_path = object_path(from);
        let to_path = object_path(to);

        match self.delete(to).await {
            Ok(_) | Err(object_store::Error::NotFound { .. }) => {}
            Err(err) => return Err(err),
        }
        self.ensure_parent_directories(&to_path).await?;

        let mut session = self.session.lock().await;
        let SmbSession { client, tree } = &mut *session;
        client
            .rename(tree, &from_path, &to_path)
            .await
            .map_err(|err| map_smb_error(err, from_path))
    }

    async fn copy_if_not_exists(&self, from: &Path, to: &Path) -> Result<()> {
        let to_path = object_path(to);
        match self.stat_info(&to_path).await {
            Ok(_) => {
                return Err(object_store::Error::AlreadyExists {
                    path: to_path,
                    source: "SMB path already exists".into(),
                });
            }
            Err(object_store::Error::NotFound { .. }) => {}
            Err(err) => return Err(err),
        }

        self.copy(from, to).await
    }

    async fn rename_if_not_exists(&self, from: &Path, to: &Path) -> Result<()> {
        let to_path = object_path(to);
        match self.stat_info(&to_path).await {
            Ok(_) => {
                return Err(object_store::Error::AlreadyExists {
                    path: to_path,
                    source: "SMB path already exists".into(),
                });
            }
            Err(object_store::Error::NotFound { .. }) => {}
            Err(err) => return Err(err),
        }

        self.ensure_parent_directories(&object_path(to)).await?;
        let from_path = object_path(from);
        let to_path = object_path(to);
        let mut session = self.session.lock().await;
        let SmbSession { client, tree } = &mut *session;
        client
            .rename(tree, &from_path, &to_path)
            .await
            .map_err(|err| map_smb_error(err, from_path))
    }
}

#[derive(Debug)]
struct SmbMultipartUpload {
    store: SmbObjectStore,
    location: Path,
    parts: Vec<PutPayload>,
}

#[async_trait]
impl MultipartUpload for SmbMultipartUpload {
    fn put_part(&mut self, data: PutPayload) -> object_store::UploadPart {
        self.parts.push(data);
        Box::pin(async { Ok(()) })
    }

    async fn complete(&mut self) -> Result<PutResult> {
        let mut data = Vec::with_capacity(self.parts.iter().map(PutPayload::content_length).sum());
        for payload in self.parts.drain(..) {
            for chunk in payload {
                data.extend_from_slice(&chunk);
            }
        }

        self.store
            .put(&self.location, PutPayload::from_bytes(Bytes::from(data)))
            .await
    }

    async fn abort(&mut self) -> Result<()> {
        self.parts.clear();
        Ok(())
    }
}

fn object_path(location: &Path) -> String {
    location
        .as_ref()
        .replace('\\', "/")
        .trim_matches('/')
        .to_string()
}

fn parent_paths(path: &str) -> Vec<String> {
    let mut current = String::new();
    let mut parents = Vec::new();
    let mut parts = path.split('/').filter(|part| !part.is_empty()).peekable();

    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            break;
        }
        if !current.is_empty() {
            current.push('/');
        }
        current.push_str(part);
        parents.push(current.clone());
    }

    parents
}

fn join_path(parent: &str, child: &str) -> String {
    let parent = parent.trim_matches('/');
    let child = child.trim_matches('/');
    if parent.is_empty() {
        child.to_string()
    } else if child.is_empty() {
        parent.to_string()
    } else {
        format!("{parent}/{child}")
    }
}

fn is_real_entry(name: &str) -> bool {
    name != "." && name != ".." && !name.is_empty()
}

fn meta_from_info(path: &str, info: &FileInfo) -> ObjectMeta {
    let last_modified = info
        .modified
        .to_system_time()
        .map(DateTime::<Utc>::from)
        .unwrap_or_else(Utc::now);
    ObjectMeta {
        location: Path::from(path),
        last_modified,
        size: info.size,
        e_tag: Some(etag(info.size, last_modified)),
        version: None,
    }
}

fn meta_from_entry(path: &str, entry: &DirectoryEntry) -> ObjectMeta {
    let last_modified = entry
        .modified
        .to_system_time()
        .map(DateTime::<Utc>::from)
        .unwrap_or_else(Utc::now);
    ObjectMeta {
        location: Path::from(path),
        last_modified,
        size: entry.size,
        e_tag: Some(etag(entry.size, last_modified)),
        version: None,
    }
}

fn etag(size: u64, modified: DateTime<Utc>) -> String {
    let modified = modified
        .timestamp_nanos_opt()
        .unwrap_or_else(|| modified.timestamp_millis() * 1_000_000);
    format!("{size:x}-{modified:x}")
}

fn payload_to_bytes(payload: PutPayload) -> Bytes {
    if payload.as_ref().len() == 1 {
        return payload.into_iter().next().unwrap_or_else(Bytes::new);
    }

    let mut data = Vec::with_capacity(payload.content_length());
    for chunk in payload {
        data.extend_from_slice(&chunk);
    }
    Bytes::from(data)
}

fn reject_attributes(attributes: &Attributes) -> Result<()> {
    if attributes.is_empty() {
        return Ok(());
    }

    Err(object_store::Error::NotSupported {
        source: "SMB object store does not support object attributes".into(),
    })
}

fn map_smb_error(err: smb2::Error, path: String) -> object_store::Error {
    match err.kind() {
        ErrorKind::NotFound => object_store::Error::NotFound {
            path,
            source: Box::new(err),
        },
        ErrorKind::AlreadyExists => object_store::Error::AlreadyExists {
            path,
            source: Box::new(err),
        },
        ErrorKind::AccessDenied => object_store::Error::PermissionDenied {
            path,
            source: Box::new(err),
        },
        ErrorKind::AuthRequired | ErrorKind::SigningRequired => {
            object_store::Error::Unauthenticated {
                path,
                source: Box::new(err),
            }
        }
        _ => object_store::Error::Generic {
            store: STORE_NAME,
            source: Box::new(err),
        },
    }
}

fn generic_error(message: impl Into<String>) -> object_store::Error {
    object_store::Error::Generic {
        store: STORE_NAME,
        source: message.into().into(),
    }
}

fn range_error(path: &str) -> object_store::Error {
    object_store::Error::Generic {
        store: STORE_NAME,
        source: format!("Invalid SMB object range for {path}").into(),
    }
}
