use chrono::{DateTime, Utc};
use flow_like_types::{Bytes, async_trait, sync::Mutex};
use futures::{StreamExt, stream};
use object_store::path::Path;
use object_store::{
    Attributes, GetOptions, GetResult, GetResultPayload, ListResult, MultipartUpload, ObjectMeta,
    ObjectStore, PutMode, PutMultipartOptions, PutOptions, PutPayload, PutResult, Result,
};
use smb2::auth::kerberos::ccache::load_ccache;
use smb2::client::{Cipher, Connection, Session};
use smb2::{
    ClientConfig, DirectoryEntry, ErrorKind, FileInfo, KerberosCredentials, SmbClient, Tree,
};
use std::collections::HashMap;
use std::fmt::{Debug, Display};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

const STORE_NAME: &str = "SMB";

#[derive(Clone)]
pub struct SmbConfig {
    pub addr: String,
    pub share: String,
    pub username: String,
    pub password: String,
    pub domain: String,
    pub auth: SmbAuth,
    pub timeout: Duration,
    pub auto_reconnect: bool,
    pub compression: bool,
    pub dfs_enabled: bool,
}

#[derive(Clone, Debug)]
pub enum SmbAuth {
    Credentials,
    KerberosCcache(SmbKerberosCcacheConfig),
}

#[derive(Clone, Debug, Default)]
pub struct SmbKerberosCcacheConfig {
    pub username: String,
    pub realm: String,
    pub kdc_address: String,
    pub ccache_path: String,
    pub server_hostname: String,
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
            auth: SmbAuth::Credentials,
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
            .field("auth", &self.auth)
            .field("timeout", &self.timeout)
            .field("auto_reconnect", &self.auto_reconnect)
            .field("compression", &self.compression)
            .field("dfs_enabled", &self.dfs_enabled)
            .finish()
    }
}

enum SmbSession {
    Client {
        client: SmbClient,
        tree: Tree,
    },
    Kerberos {
        conn: Connection,
        _session: Session,
        tree: Tree,
    },
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

        let connect_path = format!("//{}/{}", config.addr, config.share);
        let session = match &config.auth {
            SmbAuth::Credentials => connect_credentials_session(&config).await,
            SmbAuth::KerberosCcache(kerberos) => {
                connect_kerberos_ccache_session(&config, kerberos).await
            }
        }
        .map_err(|err| map_smb_error(err, connect_path))?;

        Ok(Self {
            config,
            session: Arc::new(Mutex::new(session)),
        })
    }

    async fn stat_info(&self, path: &str) -> Result<FileInfo> {
        let mut session = self.session.lock().await;
        session
            .stat(path)
            .await
            .map_err(|err| map_smb_error(err, path.to_string()))
    }

    async fn ensure_parent_directories(&self, path: &str) -> Result<()> {
        for parent in parent_paths(path) {
            let mut session = self.session.lock().await;
            match session.create_directory(&parent).await {
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
                session
                    .list_directory(&dir)
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

impl SmbSession {
    async fn stat(&mut self, path: &str) -> smb2::Result<FileInfo> {
        let path = to_smb_path(path);
        match self {
            SmbSession::Client { client, tree } => client.stat(tree, &path).await,
            SmbSession::Kerberos { conn, tree, .. } => tree.stat(conn, &path).await,
        }
    }

    async fn create_directory(&mut self, path: &str) -> smb2::Result<()> {
        let path = to_smb_path(path);
        match self {
            SmbSession::Client { client, tree } => client.create_directory(tree, &path).await,
            SmbSession::Kerberos { conn, tree, .. } => tree.create_directory(conn, &path).await,
        }
    }

    async fn list_directory(&mut self, path: &str) -> smb2::Result<Vec<DirectoryEntry>> {
        let path = to_smb_path(path);
        match self {
            SmbSession::Client { client, tree } => client.list_directory(tree, &path).await,
            SmbSession::Kerberos { conn, tree, .. } => tree.list_directory(conn, &path).await,
        }
    }

    async fn write_payload_streamed(
        &mut self,
        path: &str,
        payload: PutPayload,
    ) -> smb2::Result<u64> {
        let path = to_smb_path(path);
        let mut chunks = payload.into_iter();
        let mut next_chunk = || chunks.next().map(|chunk| Ok(chunk.to_vec()));

        match self {
            SmbSession::Client { client, tree } => {
                client
                    .write_file_streamed(tree, &path, &mut next_chunk)
                    .await
            }
            SmbSession::Kerberos { conn, tree, .. } => {
                tree.write_file_streamed(conn, &path, &mut next_chunk).await
            }
        }
    }

    async fn read_file_pipelined(&mut self, path: &str) -> smb2::Result<Vec<u8>> {
        let path = to_smb_path(path);
        match self {
            SmbSession::Client { client, tree } => client.read_file_pipelined(tree, &path).await,
            SmbSession::Kerberos { conn, tree, .. } => tree.read_file_pipelined(conn, &path).await,
        }
    }

    async fn delete_directory(&mut self, path: &str) -> smb2::Result<()> {
        let path = to_smb_path(path);
        match self {
            SmbSession::Client { client, tree } => client.delete_directory(tree, &path).await,
            SmbSession::Kerberos { conn, tree, .. } => tree.delete_directory(conn, &path).await,
        }
    }

    async fn delete_file(&mut self, path: &str) -> smb2::Result<()> {
        let path = to_smb_path(path);
        match self {
            SmbSession::Client { client, tree } => client.delete_file(tree, &path).await,
            SmbSession::Kerberos { conn, tree, .. } => tree.delete_file(conn, &path).await,
        }
    }

    async fn rename(&mut self, from: &str, to: &str) -> smb2::Result<()> {
        let from = to_smb_path(from);
        let to = to_smb_path(to);
        match self {
            SmbSession::Client { client, tree } => client.rename(tree, &from, &to).await,
            SmbSession::Kerberos { conn, tree, .. } => tree.rename(conn, &from, &to).await,
        }
    }
}

async fn connect_credentials_session(config: &SmbConfig) -> smb2::Result<SmbSession> {
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

    let mut client = SmbClient::connect(client_config).await?;
    let tree = client.connect_share(&config.share).await?;

    Ok(SmbSession::Client { client, tree })
}

async fn connect_kerberos_ccache_session(
    config: &SmbConfig,
    kerberos: &SmbKerberosCcacheConfig,
) -> smb2::Result<SmbSession> {
    let ccache_path = kerberos_ccache_path(&kerberos.ccache_path);
    let ccache = load_ccache(ccache_path.as_deref())?;
    let username = trimmed_or_default(
        &kerberos.username,
        ccache.default_principal.components.first().cloned(),
        "Kerberos username is required when the ccache default principal has no name component",
    )?;
    let realm = trimmed_or_default(
        &kerberos.realm,
        Some(ccache.default_principal.realm.clone()),
        "Kerberos realm is required when the ccache default principal has no realm",
    )?;
    let server_hostname = trimmed_or_else(&kerberos.server_hostname, || {
        host_without_port(&config.addr).to_string()
    });

    let credentials = KerberosCredentials {
        username,
        password: String::new(),
        realm,
        kdc_address: kerberos.kdc_address.trim().to_string(),
    };

    let mut conn = Connection::connect(&config.addr, config.timeout).await?;
    conn.set_compression_requested(config.compression);
    conn.negotiate().await?;

    let session =
        Session::setup_kerberos_from_ccache(&mut conn, &credentials, &server_hostname, &ccache)
            .await?;
    let tree = Tree::connect(&mut conn, &config.share).await?;
    activate_share_encryption(&mut conn, &session, &tree);

    Ok(SmbSession::Kerberos {
        conn,
        _session: session,
        tree,
    })
}

fn activate_share_encryption(conn: &mut Connection, session: &Session, tree: &Tree) {
    if !tree.encrypt_data || conn.should_encrypt() {
        return;
    }

    if let (Some(enc_key), Some(dec_key)) = (&session.encryption_key, &session.decryption_key) {
        let cipher = conn
            .params()
            .and_then(|params| params.cipher)
            .unwrap_or(Cipher::Aes128Ccm);
        conn.activate_encryption(enc_key.clone(), dec_key.clone(), cipher);
    }
}

fn kerberos_ccache_path(path: &str) -> Option<PathBuf> {
    let path = path.trim();
    if path.is_empty() {
        return None;
    }

    Some(PathBuf::from(path.strip_prefix("FILE:").unwrap_or(path)))
}

fn trimmed_or_default(
    value: &str,
    default: Option<String>,
    missing_message: &str,
) -> smb2::Result<String> {
    let value = trimmed_option(value).or_else(|| default.and_then(|value| trimmed_option(&value)));
    value.ok_or_else(|| smb2::Error::invalid_data(missing_message))
}

fn trimmed_or_else(value: &str, default: impl FnOnce() -> String) -> String {
    trimmed_option(value).unwrap_or_else(default)
}

fn trimmed_option(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn host_without_port(address: &str) -> &str {
    if let Some(rest) = address.strip_prefix('[') {
        if let Some((host, _)) = rest.split_once(']') {
            return host;
        }
    }

    if address.chars().filter(|&c| c == ':').count() != 1 {
        return address;
    }

    address
        .rsplit_once(':')
        .map(|(host, _)| host)
        .unwrap_or(address)
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
        {
            let mut session = self.session.lock().await;
            session
                .write_payload_streamed(&path, payload)
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
        _location: &Path,
        opts: PutMultipartOptions,
    ) -> Result<Box<dyn MultipartUpload>> {
        reject_attributes(&opts.attributes)?;
        Err(object_store::Error::NotSupported {
            source: "SMB multipart uploads are not supported; regular puts stream payload chunks"
                .into(),
        })
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
            session
                .read_file_pipelined(&path)
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

        if info.is_directory {
            session
                .delete_directory(&path)
                .await
                .map_err(|err| map_smb_error(err, path))
        } else {
            session
                .delete_file(&path)
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
            session
                .list_directory(&prefix)
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
        if from_path == to_path {
            return Ok(());
        }

        match self.delete(to).await {
            Ok(_) | Err(object_store::Error::NotFound { .. }) => {}
            Err(err) => return Err(err),
        }
        self.ensure_parent_directories(&to_path).await?;

        let mut session = self.session.lock().await;
        session
            .rename(&from_path, &to_path)
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
        session
            .rename(&from_path, &to_path)
            .await
            .map_err(|err| map_smb_error(err, from_path))
    }
}

fn object_path(location: &Path) -> String {
    location
        .as_ref()
        .replace('\\', "/")
        .trim_matches('/')
        .to_string()
}

fn to_smb_path(path: &str) -> String {
    path.replace('/', "\\")
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
    let last_modified = filetime_to_datetime(info.modified);
    ObjectMeta {
        location: Path::from(path),
        last_modified,
        size: info.size,
        e_tag: Some(etag(info.size, last_modified)),
        version: None,
    }
}

fn meta_from_entry(path: &str, entry: &DirectoryEntry) -> ObjectMeta {
    let last_modified = filetime_to_datetime(entry.modified);
    ObjectMeta {
        location: Path::from(path),
        last_modified,
        size: entry.size,
        e_tag: Some(etag(entry.size, last_modified)),
        version: None,
    }
}

fn filetime_to_datetime(filetime: smb2::pack::FileTime) -> DateTime<Utc> {
    filetime
        .to_system_time()
        .map(DateTime::<Utc>::from)
        .unwrap_or_else(|| DateTime::<Utc>::from(UNIX_EPOCH))
}

fn etag(size: u64, modified: DateTime<Utc>) -> String {
    let modified = modified
        .timestamp_nanos_opt()
        .unwrap_or_else(|| modified.timestamp_millis() * 1_000_000);
    format!("{size:x}-{modified:x}")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_smb_path_uses_backslash_separators() {
        assert_eq!(to_smb_path("dir/file.txt"), "dir\\file.txt");
        assert_eq!(to_smb_path("dir\\file.txt"), "dir\\file.txt");
        assert_eq!(to_smb_path(""), "");
    }

    #[test]
    fn test_object_path_normalizes_for_object_store_keys() {
        assert_eq!(object_path(&Path::from("/dir/file.txt")), "dir/file.txt");
        assert_eq!(object_path(&Path::from("dir/file.txt")), "dir/file.txt");
    }

    #[test]
    fn test_host_without_port_handles_ipv6() {
        assert_eq!(host_without_port("server:445"), "server");
        assert_eq!(host_without_port("[::1]:445"), "::1");
        assert_eq!(host_without_port("fe80::1"), "fe80::1");
    }

    #[test]
    fn test_filetime_to_datetime_uses_stable_fallback() {
        let fallback = filetime_to_datetime(smb2::pack::FileTime::ZERO);
        assert_eq!(fallback, DateTime::<Utc>::from(UNIX_EPOCH));
        assert_eq!(etag(16, fallback), etag(16, fallback));
    }
}
