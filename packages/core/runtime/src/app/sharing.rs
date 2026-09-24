use super::{App, AppVisibility};
use crate::state::FlowLikeState;
use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::{
    KeyInit, XChaCha20Poly1305,
    aead::{
        Payload,
        generic_array::GenericArray,
        stream::{DecryptorBE32, EncryptorBE32},
    },
};
use flow_like_storage::{
    Path, blake3, join_object_path,
    object_store::{ObjectStore, ObjectStoreExt, PutPayload},
};
use flow_like_types::{
    Bytes, anyhow, bail,
    rand::{TryRngCore, rngs::OsRng},
    tokio::{
        sync::{OwnedSemaphorePermit, Semaphore, mpsc},
        task,
    },
    tokio_util::sync::CancellationToken,
    utils::constant_time_eq,
};
use futures::{StreamExt, TryStreamExt, stream};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs::File,
    io::{self, Cursor, Read, Seek, SeekFrom, Write},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime},
};
use tempfile::tempfile;
use zeroize::{Zeroize, Zeroizing};
use zip::{
    CompressionMethod, ZipArchive, ZipWriter,
    write::{ExtendedFileOptions, FileOptions},
};

#[cfg(feature = "flow-runtime")]
pub mod device;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum PathKind {
    Prio,
    Secondary,
}

fn classify_path(path: &str) -> PathKind {
    if path == "manifest.app"
        || path.ends_with("/manifest.app")
        || path.ends_with(".meta")
        || path.ends_with(".template")
        || path.ends_with(".board")
    {
        return PathKind::Prio;
    }

    PathKind::Secondary
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub enum StoreKind {
    Meta,
    Storage,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ManifestEntry {
    pub store: StoreKind,
    pub rel_path: String,
    pub size: u64,
    pub blake3: String,
    #[serde(default)]
    pub segment: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExportManifest {
    pub version: u32,
    pub app_id: String,
    pub created_at: u64,
    pub prio: HashMap<String, ManifestEntry>,
    pub secondary: HashMap<String, ManifestEntry>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct ExportOptions {
    pub password: Option<String>,
    pub compact_tables: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExportReport {
    pub path: PathBuf,
    pub bytes_written: u64,
    pub file_count: u64,
    pub blob_count: u64,
    pub compaction: Option<CompactionReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CompactionReport {
    pub tables: Vec<TableCompaction>,
    pub bytes_before: u64,
    pub bytes_after: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TableCompaction {
    pub name: String,
    pub ok: bool,
    pub error: Option<String>,
    pub bytes_before: u64,
    pub bytes_after: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExportPreflight {
    pub total_bytes: u64,
    pub file_count: u64,
    pub secret_variables: Vec<SecretVariableRef>,
    pub tables: Vec<TableExportStats>,
    pub reclaimable_bytes: u64,
    pub compaction_available: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SecretVariableRef {
    pub board_id: String,
    pub board_name: String,
    pub variable_id: String,
    pub variable_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TableExportStats {
    pub name: String,
    pub versions: u64,
    pub total_bytes: u64,
    pub history_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ArchivePhase {
    Compacting,
    Listing,
    Packing,
    Writing,
    Reading,
    Planning,
    Restoring,
    Cleaning,
    Finalizing,
    Done,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ArchiveProgress {
    pub phase: ArchivePhase,
    pub done_bytes: u64,
    pub total_bytes: u64,
    pub done_files: u64,
    pub total_files: u64,
}

pub type ProgressSink = Arc<dyn Fn(ArchiveProgress) + Send + Sync>;

#[derive(Clone, Default)]
pub struct ArchiveObserver {
    pub progress: Option<ProgressSink>,
    pub cancel: Option<CancellationToken>,
}

pub const ARCHIVE_CANCELLED: &str = "Archive operation cancelled";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ImportMode {
    #[default]
    Merge,
    Replace,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct ImportOptions {
    pub password: Option<String>,
    pub mode: ImportMode,
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImportReport {
    pub app: App,
    pub mode: ImportMode,
    pub restored_files: u64,
    pub skipped_files: u64,
    pub deleted_files: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ArchiveInfo {
    pub format_version: u32,
    pub encrypted: bool,
    pub app_id: Option<String>,
    pub created_at: Option<u64>,
    pub file_count: Option<u64>,
    pub total_bytes: Option<u64>,
    pub exists_locally: bool,
    pub local_visibility: Option<AppVisibility>,
}

const PROGRESS_THROTTLE: Duration = Duration::from_millis(100);

struct PhaseState {
    phase: ArchivePhase,
    total_bytes: u64,
    total_files: u64,
    last_emit: Instant,
    dirty: bool,
}

/// Throttled progress fan-out plus the cancellation probe. Every method is
/// synchronous so blocking workers can report without touching the runtime.
struct ProgressReporter {
    sink: Option<ProgressSink>,
    cancel: Option<CancellationToken>,
    state: Mutex<PhaseState>,
    done_bytes: AtomicU64,
    done_files: AtomicU64,
}

type Reporter = Arc<ProgressReporter>;

impl ProgressReporter {
    fn new(observer: ArchiveObserver) -> Reporter {
        Arc::new(Self {
            sink: observer.progress,
            cancel: observer.cancel,
            state: Mutex::new(PhaseState {
                phase: ArchivePhase::Listing,
                total_bytes: 0,
                total_files: 0,
                last_emit: Instant::now(),
                dirty: false,
            }),
            done_bytes: AtomicU64::new(0),
            done_files: AtomicU64::new(0),
        })
    }

    fn cancelled(&self) -> bool {
        self.cancel.as_ref().is_some_and(|c| c.is_cancelled())
    }

    fn check(&self) -> flow_like_types::Result<()> {
        if self.cancelled() {
            bail!(ARCHIVE_CANCELLED);
        }
        Ok(())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, PhaseState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn emit(&self, state: &mut PhaseState) {
        if let Some(sink) = &self.sink {
            sink(ArchiveProgress {
                phase: state.phase,
                done_bytes: self.done_bytes.load(Ordering::Relaxed),
                total_bytes: state.total_bytes,
                done_files: self.done_files.load(Ordering::Relaxed),
                total_files: state.total_files,
            });
        }
        state.last_emit = Instant::now();
        state.dirty = false;
    }

    fn phase(&self, phase: ArchivePhase, total_bytes: u64, total_files: u64) {
        let mut state = self.lock();
        if state.dirty {
            self.emit(&mut state);
        }
        self.done_bytes.store(0, Ordering::Relaxed);
        self.done_files.store(0, Ordering::Relaxed);
        state.phase = phase;
        state.total_bytes = total_bytes;
        state.total_files = total_files;
        self.emit(&mut state);
    }

    fn tick(&self) {
        if self.sink.is_none() {
            return;
        }
        let mut state = self.lock();
        state.dirty = true;
        if state.last_emit.elapsed() >= PROGRESS_THROTTLE {
            self.emit(&mut state);
        }
    }

    fn add_bytes(&self, n: u64) {
        self.done_bytes.fetch_add(n, Ordering::Relaxed);
        self.tick();
    }

    fn add_files(&self, n: u64) {
        self.done_files.fetch_add(n, Ordering::Relaxed);
        self.tick();
    }

    fn finish(&self) {
        let mut state = self.lock();
        if state.dirty {
            self.emit(&mut state);
        }
        state.phase = ArchivePhase::Done;
        self.emit(&mut state);
    }
}

fn is_cancelled_error(error: &flow_like_types::Error) -> bool {
    format!("{error:#}").contains(ARCHIVE_CANCELLED)
}

const MANIFEST_VERSION: u32 = 2;
const MANIFEST_ENTRY_NAME: &str = "manifest.json";

const MAGIC_V2: &[u8; 8] = b"FLOWAPP2";
const FORMAT_VERSION_V2: u16 = 2;
const FLAG_ENCRYPTED: u16 = 1;
const HEADER_FIXED_LEN: usize = 20;
const HEADER_LEN_PLAIN: u32 = 20;
const HEADER_LEN_ENCRYPTED: u32 = 48;
const CHUNK_SIZE: u32 = 1024 * 1024;
const MAX_CHUNK_SIZE: u32 = 64 * 1024 * 1024;
const STREAM_NONCE_LEN: usize = 19;
const AEAD_TAG_LEN: u64 = 16;
const KDF_SALT_LEN: usize = 16;
const DIGEST_LEN: usize = 32;
const BINDING_TAG_LEN: usize = 32;
const INDEX_ENTRY_LEN: usize = 1 + 8 + 8 + 8 + DIGEST_LEN;
const MAX_SEGMENTS: u32 = 4096;
const MAX_MANIFEST_SEGMENT_LEN: u64 = 256 * 1024 * 1024;
const SEGMENT_KIND_MANIFEST: u8 = 0;
const SEGMENT_KIND_BLOBS: u8 = 1;

const ARGON2_M_COST_KIB: u32 = 64 * 1024;
const ARGON2_T_COST: u32 = 3;
const ARGON2_P_COST: u32 = 1;
const ARGON2_MAX_M_COST_KIB: u32 = 1024 * 1024;
const ARGON2_MAX_T_COST: u32 = 64;
const ARGON2_MAX_P_COST: u32 = 64;

const BINDING_CONTEXT_V2: &[u8] = b"flow-archive|v2";
const SEGMENT_KEY_CONTEXT: &str = "flow-like app archive v2 segment key";
const BINDING_KEY_CONTEXT: &str = "flow-like app archive v2 binding key";
const BINDING_TAG_ERROR: &str = "Archive authentication failed (binding tag mismatch)";

const READ_CONCURRENCY: usize = 4;
const HEAD_CONCURRENCY: usize = 32;
const PUT_CONCURRENCY: usize = 8;
const SHARD_QUEUE_DEPTH: usize = 2;
const PERMIT_BYTES: u64 = 64 * 1024;
// Mobile jetsam limits are far below a desktop process budget, so the in-flight
// blob budget is sized per platform instead of being a flat constant.
const BUDGET_BYTES: u64 = if cfg!(any(target_os = "ios", target_os = "android")) {
    96 * 1024 * 1024
} else {
    512 * 1024 * 1024
};
const BUDGET_PERMITS: u32 = (BUDGET_BYTES / PERMIT_BYTES) as u32;
// Objects above this size are spooled through a temp file instead of being
// materialised contiguously, so a single huge blob cannot exceed the budget.
const SPOOL_THRESHOLD: u64 = BUDGET_BYTES / 4;
const SPOOL_PERMITS: u32 = (COPY_BUFFER_LEN as u64 / PERMIT_BYTES) as u32;
// Decrypting an archive segment stages its full plaintext in a temp file, so the
// number of segments staged at once is bounded independently of read workers.
const ENCRYPTED_SEGMENT_STAGING: usize = if cfg!(any(target_os = "ios", target_os = "android")) {
    1
} else {
    2
};
const ARCHIVE_SUFFIX: &str = ".flow-app";
const ENCRYPTED_ARCHIVE_SUFFIX: &str = ".enc.flow-app";
const INLINE_HASH_LIMIT: usize = 1024 * 1024;
const COPY_BUFFER_LEN: usize = 1024 * 1024;

// ===== v1 (legacy) constants, still needed to import old archives =====
const ENC_MAGIC: &[u8] = b"FLOWAPP_CHACHA2";
const SALT_LEN: usize = 16;
const XNONCE_LEN: usize = 24;
const BINDING_SALT_LEN: usize = 16;
const BINDING_CONTEXT_V1: &[u8] = b"flow-archive|v1";

type Password = Arc<Zeroizing<String>>;

fn now_unix() -> u64 {
    use std::time::UNIX_EPOCH;
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// `PathBuf::set_extension` replaces only the last dot component, so applying
/// `enc.flow-app` to a path that already carries it yields `.enc.enc.flow-app`,
/// and it truncates any user-typed dotted file name. Strip-then-append instead.
fn apply_archive_suffix(target: &mut PathBuf, encrypted: bool) {
    let wanted = if encrypted {
        ENCRYPTED_ARCHIVE_SUFFIX
    } else {
        ARCHIVE_SUFFIX
    };
    let Some(name) = target.file_name().and_then(|name| name.to_str()) else {
        target.set_extension(wanted.trim_start_matches('.'));
        return;
    };
    let stem = name
        .strip_suffix(ENCRYPTED_ARCHIVE_SUFFIX)
        .or_else(|| name.strip_suffix(ARCHIVE_SUFFIX))
        .unwrap_or(name);
    target.set_file_name(format!("{stem}{wanted}"));
}

fn blake3_hex(data: &[u8]) -> String {
    blake3::hash(data).to_hex().to_string()
}

fn app_base(app_id: &str) -> Path {
    Path::from("apps").join(app_id)
}

/// Resolves a manifest path against the restore base.
///
/// The manifest is untrusted, but traversal cannot escape `base`: every segment
/// is re-encoded, which turns `.` and `..` into the inert `%2E` and `%2E%2E` and
/// keeps an encoded delimiter inside its own segment. Only a path that names no
/// segment at all is rejected, since that would address the base itself. A
/// segment that merely decodes to `.`, `..` or whitespace is a legal object key
/// an older release could have written, so it must still restore.
fn rebuild_path(base: &Path, rel: &str) -> flow_like_types::Result<Path> {
    let path = join_object_path(base, rel);
    if &path == base {
        bail!("Invalid archive path {rel:?}: names no path segment");
    }
    Ok(path)
}

fn shard_count() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .clamp(1, 8)
}

async fn run_blocking<T, F>(f: F) -> flow_like_types::Result<T>
where
    F: FnOnce() -> flow_like_types::Result<T> + Send + 'static,
    T: Send + 'static,
{
    task::spawn_blocking(f)
        .await
        .map_err(|e| anyhow!("Blocking task failed: {e}"))?
}

fn zip_options() -> FileOptions<'static, ExtendedFileOptions> {
    FileOptions::<ExtendedFileOptions>::default()
        .compression_method(CompressionMethod::Zstd)
        .compression_level(Some(3))
        .large_file(true)
}

#[cfg(unix)]
fn read_at(file: &File, buf: &mut [u8], offset: u64) -> io::Result<usize> {
    use std::os::unix::fs::FileExt;
    file.read_at(buf, offset)
}

#[cfg(windows)]
fn read_at(file: &File, buf: &mut [u8], offset: u64) -> io::Result<usize> {
    use std::os::windows::fs::FileExt;
    file.seek_read(buf, offset)
}

#[cfg(unix)]
fn write_all_at(file: &File, buf: &[u8], offset: u64) -> io::Result<()> {
    use std::os::unix::fs::FileExt;
    file.write_all_at(buf, offset)
}

#[cfg(windows)]
fn write_all_at(file: &File, mut buf: &[u8], mut offset: u64) -> io::Result<()> {
    use std::os::windows::fs::FileExt;
    while !buf.is_empty() {
        let n = file.seek_write(buf, offset)?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "failed to write whole buffer",
            ));
        }
        buf = &buf[n..];
        offset += n as u64;
    }
    Ok(())
}

// ===== Container primitives =====

#[derive(Clone, Copy)]
struct KdfParams {
    salt: [u8; KDF_SALT_LEN],
    m_cost: u32,
    t_cost: u32,
    p_cost: u32,
}

#[derive(Clone, Copy)]
struct HeaderV2 {
    chunk_size: u32,
    kdf: Option<KdfParams>,
}

impl HeaderV2 {
    fn encrypted(&self) -> bool {
        self.kdf.is_some()
    }

    fn to_bytes(self) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER_LEN_ENCRYPTED as usize);
        out.extend_from_slice(MAGIC_V2);
        out.extend_from_slice(&FORMAT_VERSION_V2.to_le_bytes());
        let flags = if self.encrypted() { FLAG_ENCRYPTED } else { 0 };
        out.extend_from_slice(&flags.to_le_bytes());
        let header_len = if self.encrypted() {
            HEADER_LEN_ENCRYPTED
        } else {
            HEADER_LEN_PLAIN
        };
        out.extend_from_slice(&header_len.to_le_bytes());
        out.extend_from_slice(&self.chunk_size.to_le_bytes());
        if let Some(kdf) = &self.kdf {
            out.extend_from_slice(&kdf.salt);
            out.extend_from_slice(&kdf.m_cost.to_le_bytes());
            out.extend_from_slice(&kdf.t_cost.to_le_bytes());
            out.extend_from_slice(&kdf.p_cost.to_le_bytes());
        }
        out
    }
}

#[derive(Clone, Copy)]
struct SegmentEntry {
    kind: u8,
    offset: u64,
    plain_len: u64,
    stored_len: u64,
    digest: [u8; DIGEST_LEN],
}

fn encode_index(entries: &[SegmentEntry]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + entries.len() * INDEX_ENTRY_LEN);
    out.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    for e in entries {
        out.push(e.kind);
        out.extend_from_slice(&e.offset.to_le_bytes());
        out.extend_from_slice(&e.plain_len.to_le_bytes());
        out.extend_from_slice(&e.stored_len.to_le_bytes());
        out.extend_from_slice(&e.digest);
    }
    out
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().expect("bounds checked"))
}

fn u64_at(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().expect("bounds checked"))
}

fn decode_index(bytes: &[u8]) -> flow_like_types::Result<Vec<SegmentEntry>> {
    if bytes.len() < 4 {
        bail!("Archive index is truncated");
    }
    let count = u32_at(bytes, 0);
    if count == 0 || count > MAX_SEGMENTS {
        bail!("Archive index declares {count} segments (limit {MAX_SEGMENTS})");
    }
    let expected = 4 + count as usize * INDEX_ENTRY_LEN;
    if bytes.len() != expected {
        bail!("Archive index length mismatch");
    }
    let mut entries = Vec::with_capacity(count as usize);
    for i in 0..count as usize {
        let at = 4 + i * INDEX_ENTRY_LEN;
        let kind = bytes[at];
        if kind != SEGMENT_KIND_MANIFEST && kind != SEGMENT_KIND_BLOBS {
            bail!("Archive segment {i} has unknown kind {kind}");
        }
        let mut digest = [0u8; DIGEST_LEN];
        digest.copy_from_slice(&bytes[at + 25..at + 25 + DIGEST_LEN]);
        entries.push(SegmentEntry {
            kind,
            offset: u64_at(bytes, at + 1),
            plain_len: u64_at(bytes, at + 9),
            stored_len: u64_at(bytes, at + 17),
            digest,
        });
    }
    Ok(entries)
}

struct ArchiveV2 {
    header: HeaderV2,
    header_bytes: Vec<u8>,
    index_bytes: Vec<u8>,
    segments: Vec<SegmentEntry>,
    tag: Option<[u8; BINDING_TAG_LEN]>,
}

fn read_v2_layout(file: &mut File) -> flow_like_types::Result<ArchiveV2> {
    let total = file.metadata()?.len();
    let mut fixed = [0u8; HEADER_FIXED_LEN];
    file.seek(SeekFrom::Start(0))?;
    file.read_exact(&mut fixed)
        .map_err(|_| anyhow!("Archive is truncated (header)"))?;
    if &fixed[0..8] != MAGIC_V2 {
        bail!("Not a flow-app v2 archive");
    }
    let version = u16::from_le_bytes([fixed[8], fixed[9]]);
    if version != FORMAT_VERSION_V2 {
        bail!("Unsupported archive format version {version}");
    }
    let flags = u16::from_le_bytes([fixed[10], fixed[11]]);
    let encrypted = flags & FLAG_ENCRYPTED != 0;
    let header_len = u32_at(&fixed, 12);
    let chunk_size = u32_at(&fixed, 16);
    if chunk_size == 0 || chunk_size > MAX_CHUNK_SIZE {
        bail!("Archive chunk size {chunk_size} is out of range");
    }
    let min_header = if encrypted {
        HEADER_LEN_ENCRYPTED
    } else {
        HEADER_LEN_PLAIN
    };
    if header_len < min_header {
        bail!("Archive header length {header_len} is too small");
    }
    let trailer_len = 8 + if encrypted { BINDING_TAG_LEN as u64 } else { 0 };
    if (header_len as u64)
        .saturating_add(trailer_len)
        .saturating_add(4)
        > total
    {
        bail!("Archive is truncated (trailer)");
    }

    let mut header_bytes = vec![0u8; header_len as usize];
    file.seek(SeekFrom::Start(0))?;
    file.read_exact(&mut header_bytes)?;
    let kdf = encrypted.then(|| {
        let mut salt = [0u8; KDF_SALT_LEN];
        salt.copy_from_slice(&header_bytes[20..36]);
        KdfParams {
            salt,
            m_cost: u32_at(&header_bytes, 36),
            t_cost: u32_at(&header_bytes, 40),
            p_cost: u32_at(&header_bytes, 44),
        }
    });
    if let Some(kdf) = &kdf
        && (kdf.m_cost > ARGON2_MAX_M_COST_KIB
            || kdf.t_cost > ARGON2_MAX_T_COST
            || kdf.p_cost > ARGON2_MAX_P_COST)
    {
        bail!("Archive KDF parameters are out of range");
    }

    file.seek(SeekFrom::Start(total - trailer_len))?;
    let mut index_len_bytes = [0u8; 8];
    file.read_exact(&mut index_len_bytes)?;
    let index_len = u64::from_le_bytes(index_len_bytes);
    let tag = if encrypted {
        let mut tag = [0u8; BINDING_TAG_LEN];
        file.read_exact(&mut tag)?;
        Some(tag)
    } else {
        None
    };

    let max_index_len = 4 + MAX_SEGMENTS as u64 * INDEX_ENTRY_LEN as u64;
    if index_len < 4 || index_len > max_index_len {
        bail!("Archive index length {index_len} is out of range");
    }
    let index_start = (total - trailer_len)
        .checked_sub(index_len)
        .filter(|start| *start >= header_len as u64)
        .ok_or_else(|| anyhow!("Archive index does not fit the file"))?;
    let mut index_bytes = vec![0u8; index_len as usize];
    file.seek(SeekFrom::Start(index_start))?;
    file.read_exact(&mut index_bytes)?;
    let segments = decode_index(&index_bytes)?;

    for (i, seg) in segments.iter().enumerate() {
        let end = seg
            .offset
            .checked_add(seg.stored_len)
            .ok_or_else(|| anyhow!("Archive segment {i} overflows"))?;
        if seg.offset < header_len as u64 || end > index_start {
            bail!("Archive segment {i} lies outside the file");
        }
        let expected_stored = if encrypted {
            stored_len_for(seg.plain_len, chunk_size)
        } else {
            seg.plain_len
        };
        if seg.stored_len != expected_stored {
            bail!("Archive segment {i} has inconsistent lengths");
        }
    }
    if segments[0].kind != SEGMENT_KIND_MANIFEST {
        bail!("Archive has no manifest segment");
    }

    Ok(ArchiveV2 {
        header: HeaderV2 { chunk_size, kdf },
        header_bytes,
        index_bytes,
        segments,
        tag,
    })
}

// ===== Keys and STREAM encryption =====

struct ArchiveKeys {
    segment: [u8; 32],
    binding: [u8; 32],
}

impl Drop for ArchiveKeys {
    fn drop(&mut self) {
        self.segment.zeroize();
        self.binding.zeroize();
    }
}

fn argon2_derive(
    password: &[u8],
    salt: &[u8],
    m_cost: u32,
    t_cost: u32,
    p_cost: u32,
) -> flow_like_types::Result<[u8; 32]> {
    let params = Params::new(m_cost, t_cost, p_cost, None).map_err(|e| anyhow!(e.to_string()))?;
    let kdf = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = [0u8; 32];
    kdf.hash_password_into(password, salt, &mut key)
        .map_err(|e| anyhow!(e.to_string()))?;
    Ok(key)
}

fn derive_archive_keys(password: &str, kdf: &KdfParams) -> flow_like_types::Result<ArchiveKeys> {
    let mut master = argon2_derive(
        password.as_bytes(),
        &kdf.salt,
        kdf.m_cost,
        kdf.t_cost,
        kdf.p_cost,
    )?;
    let keys = ArchiveKeys {
        segment: blake3::derive_key(SEGMENT_KEY_CONTEXT, &master),
        binding: blake3::derive_key(BINDING_KEY_CONTEXT, &master),
    };
    master.zeroize();
    Ok(keys)
}

fn binding_tag_v2(key: &[u8; 32], header: &[u8], index: &[u8]) -> [u8; BINDING_TAG_LEN] {
    let mut hasher = blake3::Hasher::new_keyed(key);
    hasher.update(BINDING_CONTEXT_V2);
    hasher.update(header);
    hasher.update(index);
    *hasher.finalize().as_bytes()
}

fn stored_len_for(plain_len: u64, chunk_size: u32) -> u64 {
    let chunks = plain_len.div_ceil(chunk_size as u64).max(1);
    STREAM_NONCE_LEN as u64 + plain_len + AEAD_TAG_LEN * chunks
}

type CancelProbe<'a> = &'a dyn Fn() -> bool;

fn check_probe(cancelled: CancelProbe<'_>) -> flow_like_types::Result<()> {
    if cancelled() {
        bail!(ARCHIVE_CANCELLED);
    }
    Ok(())
}

fn copy_segment<R: Read, W: Write>(
    src: &mut R,
    plain_len: u64,
    chunk_size: u32,
    cancelled: CancelProbe<'_>,
    out: &mut W,
) -> flow_like_types::Result<()> {
    let mut buf = vec![0u8; chunk_size as usize];
    let mut remaining = plain_len;
    while remaining > 0 {
        check_probe(cancelled)?;
        let take = remaining.min(chunk_size as u64) as usize;
        src.read_exact(&mut buf[..take])?;
        out.write_all(&buf[..take])?;
        remaining -= take as u64;
    }
    Ok(())
}

fn encrypt_segment<R: Read, W: Write>(
    src: &mut R,
    plain_len: u64,
    key: &[u8; 32],
    segment_index: u32,
    chunk_size: u32,
    cancelled: CancelProbe<'_>,
    out: &mut W,
) -> flow_like_types::Result<()> {
    let mut nonce = [0u8; STREAM_NONCE_LEN];
    OsRng.try_fill_bytes(&mut nonce)?;
    out.write_all(&nonce)?;

    let cipher = XChaCha20Poly1305::new_from_slice(key).map_err(|e| anyhow!(e))?;
    let mut encryptor = EncryptorBE32::from_aead(cipher, GenericArray::from_slice(&nonce));
    let aad = segment_index.to_le_bytes();
    let mut buf = vec![0u8; chunk_size as usize];
    let mut remaining = plain_len;
    loop {
        check_probe(cancelled)?;
        let take = remaining.min(chunk_size as u64) as usize;
        src.read_exact(&mut buf[..take])?;
        remaining -= take as u64;
        let payload = Payload {
            msg: &buf[..take],
            aad: &aad,
        };
        if remaining == 0 {
            let ct = encryptor
                .encrypt_last(payload)
                .map_err(|_| anyhow!("Segment {segment_index} encryption failed"))?;
            out.write_all(&ct)?;
            break;
        }
        let ct = encryptor
            .encrypt_next(payload)
            .map_err(|_| anyhow!("Segment {segment_index} encryption failed"))?;
        out.write_all(&ct)?;
    }
    Ok(())
}

fn decrypt_segment<R: Read, W: Write>(
    src: &mut R,
    stored_len: u64,
    key: &[u8; 32],
    segment_index: u32,
    chunk_size: u32,
    cancelled: CancelProbe<'_>,
    out: &mut W,
) -> flow_like_types::Result<u64> {
    if stored_len < STREAM_NONCE_LEN as u64 + AEAD_TAG_LEN {
        bail!("Segment {segment_index} is too short to be encrypted");
    }
    let mut nonce = [0u8; STREAM_NONCE_LEN];
    src.read_exact(&mut nonce)?;

    let cipher = XChaCha20Poly1305::new_from_slice(key).map_err(|e| anyhow!(e))?;
    let mut decryptor = DecryptorBE32::from_aead(cipher, GenericArray::from_slice(&nonce));
    let aad = segment_index.to_le_bytes();
    let max_chunk = chunk_size as u64 + AEAD_TAG_LEN;
    let mut buf = vec![0u8; max_chunk as usize];
    let mut remaining = stored_len - STREAM_NONCE_LEN as u64;
    let mut written = 0u64;
    loop {
        check_probe(cancelled)?;
        let last = remaining <= max_chunk;
        let take = if last { remaining } else { max_chunk } as usize;
        src.read_exact(&mut buf[..take])?;
        remaining -= take as u64;
        let payload = Payload {
            msg: &buf[..take],
            aad: &aad,
        };
        if last {
            let plain = decryptor
                .decrypt_last(payload)
                .map_err(|_| anyhow!("Segment {segment_index} decryption failed"))?;
            out.write_all(&plain)?;
            written += plain.len() as u64;
            break;
        }
        let plain = decryptor
            .decrypt_next(payload)
            .map_err(|_| anyhow!("Segment {segment_index} decryption failed"))?;
        out.write_all(&plain)?;
        written += plain.len() as u64;
    }
    Ok(written)
}

// ===== File views =====

struct PositionalWriter<'a> {
    file: &'a File,
    offset: u64,
    written: u64,
    hasher: blake3::Hasher,
    reporter: &'a ProgressReporter,
}

impl<'a> PositionalWriter<'a> {
    fn new(file: &'a File, offset: u64, reporter: &'a ProgressReporter) -> Self {
        Self {
            file,
            offset,
            written: 0,
            hasher: blake3::Hasher::new(),
            reporter,
        }
    }

    fn finish(self) -> (u64, [u8; DIGEST_LEN]) {
        (self.written, *self.hasher.finalize().as_bytes())
    }
}

impl Write for PositionalWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        write_all_at(self.file, buf, self.offset + self.written)?;
        self.hasher.update(buf);
        self.written += buf.len() as u64;
        self.reporter.add_bytes(buf.len() as u64);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Window over one segment of the source archive. Reads are positional so
/// several workers can share one underlying file description without racing
/// on its cursor.
struct SectionReader {
    file: File,
    start: u64,
    len: u64,
    pos: u64,
}

impl SectionReader {
    fn new(file: File, start: u64, len: u64) -> Self {
        Self {
            file,
            start,
            len,
            pos: 0,
        }
    }
}

impl Read for SectionReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.pos >= self.len {
            return Ok(0);
        }
        let max = (self.len - self.pos).min(buf.len() as u64) as usize;
        let n = read_at(&self.file, &mut buf[..max], self.start + self.pos)?;
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for SectionReader {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let target = match pos {
            SeekFrom::Start(p) => p as i128,
            SeekFrom::End(d) => self.len as i128 + d as i128,
            SeekFrom::Current(d) => self.pos as i128 + d as i128,
        };
        if target < 0 || target > u64::MAX as i128 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "seek outside of segment",
            ));
        }
        self.pos = target as u64;
        Ok(self.pos)
    }
}

enum SegmentReader {
    Plain(SectionReader),
    Temp(File),
}

impl Read for SegmentReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            SegmentReader::Plain(r) => r.read(buf),
            SegmentReader::Temp(f) => f.read(buf),
        }
    }
}

impl Seek for SegmentReader {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        match self {
            SegmentReader::Plain(r) => r.seek(pos),
            SegmentReader::Temp(f) => f.seek(pos),
        }
    }
}

fn digest_of_reader<R: Read>(reader: &mut R) -> io::Result<[u8; DIGEST_LEN]> {
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; COPY_BUFFER_LEN];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(*hasher.finalize().as_bytes())
}

fn open_segment(
    source: &File,
    entry: SegmentEntry,
    index: u32,
    keys: Option<&ArchiveKeys>,
    chunk_size: u32,
    cancelled: CancelProbe<'_>,
) -> flow_like_types::Result<SegmentReader> {
    check_probe(cancelled)?;
    let mut section = SectionReader::new(source.try_clone()?, entry.offset, entry.stored_len);
    let digest = digest_of_reader(&mut section)?;
    if !constant_time_eq(&digest, &entry.digest) {
        bail!("Archive segment {index} is corrupted (digest mismatch)");
    }
    section.seek(SeekFrom::Start(0))?;

    let Some(keys) = keys else {
        return Ok(SegmentReader::Plain(section));
    };
    let mut plain = tempfile()?;
    let written = decrypt_segment(
        &mut section,
        entry.stored_len,
        &keys.segment,
        index,
        chunk_size,
        cancelled,
        &mut plain,
    )?;
    if written != entry.plain_len {
        bail!("Archive segment {index} decrypted to an unexpected length");
    }
    plain.flush()?;
    plain.seek(SeekFrom::Start(0))?;
    Ok(SegmentReader::Temp(plain))
}

fn read_zip_entry<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    name: &str,
    max_len: u64,
) -> flow_like_types::Result<Vec<u8>> {
    let file = archive
        .by_name(name)
        .map_err(|_| anyhow!("Missing archive entry {name}"))?;
    let mut buf = Vec::with_capacity(file.size().min(max_len) as usize);
    file.take(max_len + 1).read_to_end(&mut buf)?;
    if buf.len() as u64 > max_len {
        bail!("Archive entry {name} exceeds its declared size");
    }
    Ok(buf)
}

fn parse_manifest<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    max_len: u64,
) -> flow_like_types::Result<ExportManifest> {
    let bytes = read_zip_entry(archive, MANIFEST_ENTRY_NAME, max_len)?;
    let manifest: ExportManifest = flow_like_types::json::from_slice(&bytes)?;
    if manifest.version > MANIFEST_VERSION {
        bail!(
            "Archive manifest version {} is newer than supported ({MANIFEST_VERSION})",
            manifest.version
        );
    }
    Ok(manifest)
}

// ===== Store listing =====

#[derive(Clone)]
struct ListedFile {
    store: StoreKind,
    path: Path,
    rel: String,
    size: u64,
}

async fn list_store_files(
    store: &Arc<dyn ObjectStore>,
    root: &Path,
) -> flow_like_types::Result<Vec<(Path, u64)>> {
    let mut out = Vec::new();
    let mut stream = store.list(Some(root)).boxed();
    while let Some(item) = stream.try_next().await? {
        out.push((item.location, item.size));
    }
    Ok(out)
}

async fn list_app_files(
    meta: &Arc<dyn ObjectStore>,
    storage: &Arc<dyn ObjectStore>,
    base: &Path,
) -> flow_like_types::Result<Vec<ListedFile>> {
    let prefix = format!("{base}/");
    let mut merged: HashMap<String, ListedFile> = HashMap::new();
    for (kind, store) in [(StoreKind::Meta, meta), (StoreKind::Storage, storage)] {
        for (path, size) in list_store_files(store, base).await? {
            let full = path.to_string();
            let rel = full.strip_prefix(&prefix).unwrap_or(&full).to_string();
            merged.insert(
                rel.clone(),
                ListedFile {
                    store: kind,
                    path,
                    rel,
                    size,
                },
            );
        }
    }
    Ok(merged.into_values().collect())
}

async fn prefix_bytes(store: &Arc<dyn ObjectStore>, prefix: &Path) -> flow_like_types::Result<u64> {
    Ok(list_store_files(store, prefix)
        .await?
        .iter()
        .map(|(_, size)| *size)
        .sum())
}

fn table_stats(files: &[ListedFile]) -> Vec<TableExportStats> {
    const DB_PREFIX: &str = "storage/db/";

    #[derive(Default)]
    struct Acc {
        total: u64,
        versions: u64,
        version_bytes: u64,
        max_version: u64,
        txn_bytes: u64,
        max_txn: u64,
    }

    let mut tables: BTreeMap<String, Acc> = BTreeMap::new();
    for file in files {
        let Some(rest) = file.rel.strip_prefix(DB_PREFIX) else {
            continue;
        };
        let Some((dir, inner)) = rest.split_once('/') else {
            continue;
        };
        let Some(name) = dir.strip_suffix(".lance") else {
            continue;
        };
        let acc = tables.entry(name.to_string()).or_default();
        acc.total += file.size;
        if inner.starts_with("_versions/") {
            acc.versions += 1;
            acc.version_bytes += file.size;
            acc.max_version = acc.max_version.max(file.size);
        } else if inner.starts_with("_transactions/") {
            acc.txn_bytes += file.size;
            acc.max_txn = acc.max_txn.max(file.size);
        }
    }

    tables
        .into_iter()
        .map(|(name, acc)| TableExportStats {
            name,
            versions: acc.versions,
            total_bytes: acc.total,
            history_bytes: (acc.version_bytes - acc.max_version) + (acc.txn_bytes - acc.max_txn),
        })
        .collect()
}

fn secret_has_value(bytes: Option<&[u8]>) -> bool {
    let Some(bytes) = bytes else {
        return false;
    };
    match flow_like_types::json::from_slice::<flow_like_types::Value>(bytes) {
        Ok(flow_like_types::Value::Null) => false,
        Ok(flow_like_types::Value::String(s)) => !s.is_empty(),
        Ok(_) => true,
        Err(_) => false,
    }
}

// ===== Export pipeline =====

/// Blobs below `SPOOL_THRESHOLD` stay in memory; larger ones are spooled to a
/// temp file so the export byte budget bounds a single object as well.
enum BlobBody {
    Inline(Bytes),
    Spooled(File),
}

struct ShardEntry {
    name: String,
    body: BlobBody,
    _permit: OwnedSemaphorePermit,
}

fn write_shard_zip(
    mut rx: mpsc::Receiver<ShardEntry>,
    reporter: Reporter,
) -> flow_like_types::Result<File> {
    let mut writer = ZipWriter::new(tempfile()?);
    let options = zip_options();
    while let Some(entry) = rx.blocking_recv() {
        reporter.check()?;
        writer.start_file(entry.name.as_str(), options.clone())?;
        match entry.body {
            BlobBody::Inline(bytes) => writer.write_all(&bytes)?,
            BlobBody::Spooled(mut file) => {
                io::copy(&mut file, &mut writer)?;
            }
        }
    }
    let mut file = writer.finish()?;
    file.flush()?;
    Ok(file)
}

fn write_manifest_zip(manifest: &ExportManifest) -> flow_like_types::Result<File> {
    let bytes = flow_like_types::json::to_vec(manifest)?;
    let mut writer = ZipWriter::new(tempfile()?);
    writer.start_file(MANIFEST_ENTRY_NAME, zip_options())?;
    writer.write_all(&bytes)?;
    let mut file = writer.finish()?;
    file.flush()?;
    Ok(file)
}

struct ReadBlob {
    file: ListedFile,
    body: BlobBody,
    len: u64,
    hash: String,
    permit: OwnedSemaphorePermit,
}

/// Streams an oversized object through a temp file, hashing as it goes, so peak
/// memory is one copy buffer rather than the whole object.
async fn spool_and_hash(
    store: Arc<dyn ObjectStore>,
    file: ListedFile,
    budget: Arc<Semaphore>,
    reporter: Reporter,
) -> flow_like_types::Result<ReadBlob> {
    let permit = budget
        .acquire_many_owned(SPOOL_PERMITS)
        .await
        .map_err(|_| anyhow!("Export byte budget closed"))?;
    reporter.check()?;
    let mut stream = store
        .get(&file.path)
        .await
        .map_err(|e| anyhow!("Failed to read {}: {e}", file.rel))?
        .into_stream();
    let mut spool = run_blocking(|| Ok(tempfile()?)).await?;
    let mut hasher = blake3::Hasher::new();
    let mut len = 0u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| anyhow!("Failed to read {}: {e}", file.rel))?;
        reporter.check()?;
        len += chunk.len() as u64;
        (spool, hasher) = run_blocking(move || {
            hasher.update(&chunk);
            spool.write_all(&chunk)?;
            Ok((spool, hasher))
        })
        .await?;
    }
    let spool = run_blocking(move || {
        spool.flush()?;
        spool.seek(SeekFrom::Start(0))?;
        Ok(spool)
    })
    .await?;
    // The Packing phase total is a sum of listed sizes, so progress advances by
    // the listed size even when the object changed underneath us.
    reporter.add_bytes(file.size);
    reporter.add_files(1);
    Ok(ReadBlob {
        body: BlobBody::Spooled(spool),
        file,
        len,
        hash: hasher.finalize().to_hex().to_string(),
        permit,
    })
}

async fn read_and_hash(
    store: Arc<dyn ObjectStore>,
    file: ListedFile,
    budget: Arc<Semaphore>,
    reporter: Reporter,
) -> flow_like_types::Result<ReadBlob> {
    if file.size > SPOOL_THRESHOLD {
        return spool_and_hash(store, file, budget, reporter).await;
    }
    let needed = file.size.div_ceil(PERMIT_BYTES).min(BUDGET_PERMITS as u64) as u32;
    let permit = budget
        .acquire_many_owned(needed)
        .await
        .map_err(|_| anyhow!("Export byte budget closed"))?;
    reporter.check()?;
    let bytes = store
        .get(&file.path)
        .await
        .map_err(|e| anyhow!("Failed to read {}: {e}", file.rel))?
        .bytes()
        .await
        .map_err(|e| anyhow!("Failed to read {}: {e}", file.rel))?;
    let (bytes, hash) = if bytes.len() > INLINE_HASH_LIMIT {
        run_blocking(move || {
            let hash = blake3_hex(&bytes);
            Ok((bytes, hash))
        })
        .await?
    } else {
        let hash = blake3_hex(&bytes);
        (bytes, hash)
    };
    reporter.add_bytes(file.size);
    reporter.add_files(1);
    Ok(ReadBlob {
        file,
        len: bytes.len() as u64,
        body: BlobBody::Inline(bytes),
        hash,
        permit,
    })
}

struct CollectedBlobs {
    manifest: ExportManifest,
    shards: Vec<File>,
    blob_count: u64,
}

async fn collect_blobs(
    app_id: &str,
    meta: Arc<dyn ObjectStore>,
    storage: Arc<dyn ObjectStore>,
    files: Vec<ListedFile>,
    shard_count: usize,
    reporter: Reporter,
) -> flow_like_types::Result<CollectedBlobs> {
    reporter.phase(
        ArchivePhase::Packing,
        files.iter().map(|f| f.size).sum(),
        files.len() as u64,
    );
    let mut senders = Vec::with_capacity(shard_count);
    let mut writers = Vec::with_capacity(shard_count);
    for _ in 0..shard_count {
        let (tx, rx) = mpsc::channel::<ShardEntry>(SHARD_QUEUE_DEPTH);
        let reporter = reporter.clone();
        senders.push(tx);
        writers.push(task::spawn_blocking(move || write_shard_zip(rx, reporter)));
    }

    let mut manifest = ExportManifest {
        version: MANIFEST_VERSION,
        app_id: app_id.to_string(),
        created_at: now_unix(),
        prio: HashMap::new(),
        secondary: HashMap::new(),
    };

    let pump = async {
        let senders = senders;
        let budget = Arc::new(Semaphore::new(BUDGET_PERMITS as usize));
        let mut seen: HashSet<String> = HashSet::new();
        let mut reads = stream::iter(files.into_iter().map(|file| {
            let store = match file.store {
                StoreKind::Meta => meta.clone(),
                StoreKind::Storage => storage.clone(),
            };
            read_and_hash(store, file, budget.clone(), reporter.clone())
        }))
        .buffer_unordered(READ_CONCURRENCY);

        while let Some(item) = reads.next().await {
            let ReadBlob {
                file,
                body,
                len,
                hash,
                permit,
            } = item?;
            let shard = shard_for(&hash, shard_count)?;
            if seen.insert(hash.clone()) {
                senders[shard]
                    .send(ShardEntry {
                        name: hash.clone(),
                        body,
                        _permit: permit,
                    })
                    .await
                    .map_err(|_| anyhow!("Shard writer {shard} stopped early"))?;
            }
            let entry = ManifestEntry {
                store: file.store,
                rel_path: file.rel.clone(),
                size: len,
                blake3: hash,
                segment: shard as u32 + 1,
            };
            match classify_path(&file.rel) {
                PathKind::Prio => manifest.prio.insert(file.rel, entry),
                PathKind::Secondary => manifest.secondary.insert(file.rel, entry),
            };
        }
        Ok::<u64, flow_like_types::Error>(seen.len() as u64)
    };
    let pumped = pump.await;

    let mut shards = Vec::with_capacity(shard_count);
    let mut writer_error = None;
    for (i, handle) in writers.into_iter().enumerate() {
        let outcome = match handle.await {
            Ok(Ok(file)) => Ok(file),
            Ok(Err(e)) => Err(anyhow!("Shard writer {i} failed: {e}")),
            Err(e) => Err(anyhow!("Shard writer {i} panicked: {e}")),
        };
        match outcome {
            Ok(file) => shards.push(file),
            Err(e) => {
                writer_error.get_or_insert(e);
            }
        }
    }
    let blob_count = match (pumped, writer_error) {
        (_, Some(e)) | (Err(e), None) => return Err(e),
        (Ok(count), None) => count,
    };

    Ok(CollectedBlobs {
        manifest,
        shards,
        blob_count,
    })
}

fn shard_for(hash_hex: &str, shard_count: usize) -> flow_like_types::Result<usize> {
    let raw = blake3::Hash::from_hex(hash_hex).map_err(|e| anyhow!("Invalid blob hash: {e}"))?;
    let head = u64::from_le_bytes(raw.as_bytes()[..8].try_into().expect("32-byte hash"));
    Ok((head % shard_count as u64) as usize)
}

struct SegmentSource {
    kind: u8,
    file: File,
    plain_len: u64,
}

/// A sibling of the target in the same directory, so publishing is a same-volume
/// rename. The pid/nanosecond suffix keeps two concurrent exports of one app apart.
fn staging_path(target: &std::path::Path) -> PathBuf {
    let mut name = target
        .file_name()
        .unwrap_or_else(|| std::ffi::OsStr::new("archive"))
        .to_os_string();
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|since| since.as_nanos())
        .unwrap_or_default();
    name.push(format!(".part-{}-{nanos}", std::process::id()));
    match target.parent() {
        Some(parent) => parent.join(name),
        None => PathBuf::from(name),
    }
}

async fn assemble_archive(
    target: PathBuf,
    header: HeaderV2,
    sources: Vec<SegmentSource>,
    keys: Option<Arc<ArchiveKeys>>,
    workers: usize,
    reporter: Reporter,
) -> flow_like_types::Result<u64> {
    let header_bytes = header.to_bytes();
    let mut entries = Vec::with_capacity(sources.len());
    let mut offset = header_bytes.len() as u64;
    for source in &sources {
        let stored_len = if header.encrypted() {
            stored_len_for(source.plain_len, header.chunk_size)
        } else {
            source.plain_len
        };
        entries.push(SegmentEntry {
            kind: source.kind,
            offset,
            plain_len: source.plain_len,
            stored_len,
            digest: [0u8; DIGEST_LEN],
        });
        offset += stored_len;
    }
    let segments_end = offset;
    reporter.phase(
        ArchivePhase::Writing,
        entries.iter().map(|e| e.stored_len).sum(),
        entries.len() as u64,
    );
    reporter.check()?;

    // Never open the caller's path until the archive is complete: File::create
    // truncates, so a failure mid-write would destroy a pre-existing archive.
    let staging = staging_path(&target);

    let out = {
        let staging = staging.clone();
        let header_bytes = header_bytes.clone();
        run_blocking(move || {
            let out = File::create(&staging)?;
            out.set_len(segments_end)?;
            write_all_at(&out, &header_bytes, 0)?;
            Ok(Arc::new(out))
        })
        .await?
    };

    let result = write_archive_body(
        out,
        header.chunk_size,
        header_bytes,
        entries,
        sources,
        keys,
        workers,
        segments_end,
        reporter,
    )
    .await;

    let bytes_written = match result {
        Ok(bytes_written) => bytes_written,
        Err(e) => {
            let _ = std::fs::remove_file(&staging);
            return Err(e);
        }
    };

    let published = {
        let staging = staging.clone();
        let target = target.clone();
        run_blocking(move || Ok(std::fs::rename(&staging, &target)?)).await
    };
    if let Err(e) = published {
        let _ = std::fs::remove_file(&staging);
        bail!("Failed to publish archive to {}: {e}", target.display());
    }
    Ok(bytes_written)
}

#[allow(clippy::too_many_arguments)]
async fn write_archive_body(
    out: Arc<File>,
    chunk_size: u32,
    header_bytes: Vec<u8>,
    mut entries: Vec<SegmentEntry>,
    sources: Vec<SegmentSource>,
    keys: Option<Arc<ArchiveKeys>>,
    workers: usize,
    segments_end: u64,
    reporter: Reporter,
) -> flow_like_types::Result<u64> {
    let digests: Vec<(usize, u64, [u8; DIGEST_LEN])> =
        stream::iter(sources.into_iter().enumerate().map(|(i, source)| {
            let out = out.clone();
            let keys = keys.clone();
            let entry = entries[i];
            let reporter = reporter.clone();
            task::spawn_blocking(move || -> flow_like_types::Result<_> {
                let mut src = source.file.try_clone()?;
                src.seek(SeekFrom::Start(0))?;
                let cancelled = || reporter.cancelled();
                let mut writer = PositionalWriter::new(&out, entry.offset, &reporter);
                match &keys {
                    Some(keys) => encrypt_segment(
                        &mut src,
                        entry.plain_len,
                        &keys.segment,
                        i as u32,
                        chunk_size,
                        &cancelled,
                        &mut writer,
                    )?,
                    None => copy_segment(
                        &mut src,
                        entry.plain_len,
                        chunk_size,
                        &cancelled,
                        &mut writer,
                    )
                    .map_err(|e| anyhow!("Segment {i} copy failed: {e}"))?,
                }
                let (written, digest) = writer.finish();
                reporter.add_files(1);
                Ok((i, written, digest))
            })
        }))
        .buffer_unordered(workers)
        .map(|joined| joined.map_err(|e| anyhow!("Segment worker panicked: {e}"))?)
        .try_collect()
        .await?;

    for (i, written, digest) in digests {
        if written != entries[i].stored_len {
            bail!(
                "Segment {i} wrote {written} bytes, expected {}",
                entries[i].stored_len
            );
        }
        entries[i].digest = digest;
    }

    reporter.phase(ArchivePhase::Finalizing, 0, 0);
    reporter.check()?;
    let index_bytes = encode_index(&entries);
    let tag = keys
        .as_ref()
        .map(|keys| binding_tag_v2(&keys.binding, &header_bytes, &index_bytes));
    run_blocking(move || {
        let mut tail = out.try_clone()?;
        tail.seek(SeekFrom::Start(segments_end))?;
        tail.write_all(&index_bytes)?;
        tail.write_all(&(index_bytes.len() as u64).to_le_bytes())?;
        if let Some(tag) = tag {
            tail.write_all(&tag)?;
        }
        tail.flush()?;
        tail.sync_all()?;
        Ok(tail.metadata()?.len())
    })
    .await
}

// ===== Restore helpers shared by v1 and v2 =====

struct RestoreTargets {
    meta: Arc<dyn ObjectStore>,
    storage: Arc<dyn ObjectStore>,
    base: Path,
}

impl RestoreTargets {
    fn store(&self, kind: StoreKind) -> &Arc<dyn ObjectStore> {
        match kind {
            StoreKind::Meta => &self.meta,
            StoreKind::Storage => &self.storage,
        }
    }

    fn target(&self, rel: &str) -> flow_like_types::Result<Path> {
        rebuild_path(&self.base, rel)
    }
}

type SegmentPlan = HashMap<u32, HashMap<String, Vec<String>>>;
type PlannedEntries = Vec<(u32, ManifestEntry)>;
type EntriesByRel = Arc<HashMap<String, ManifestEntry>>;

async fn stream_blake3_of_object(
    store: &Arc<dyn ObjectStore>,
    path: &Path,
) -> flow_like_types::Result<String> {
    let mut stream = store.get(path).await?.into_stream();
    let mut hasher = blake3::Hasher::new();
    while let Some(chunk) = stream.try_next().await? {
        hasher.update(&chunk);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

async fn existing_matches(
    store: &Arc<dyn ObjectStore>,
    path: &Path,
    size: u64,
    hash: &str,
) -> flow_like_types::Result<bool> {
    match store.head(path).await {
        Ok(meta) if meta.size == size => Ok(stream_blake3_of_object(store, path).await? == hash),
        Ok(_) | Err(_) => Ok(false),
    }
}

fn split_manifest(manifest: ExportManifest, legacy: bool) -> (PlannedEntries, EntriesByRel) {
    let prio = manifest
        .prio
        .into_values()
        .map(|e| (if legacy { 1 } else { e.segment }, e));
    let secondary = manifest
        .secondary
        .into_values()
        .map(|e| (if legacy { 2 } else { e.segment }, e));
    let planned: Vec<(u32, ManifestEntry)> = prio.chain(secondary).collect();
    let by_rel = planned
        .iter()
        .map(|(_, e)| (e.rel_path.clone(), e.clone()))
        .collect();
    (planned, Arc::new(by_rel))
}

#[derive(Default)]
struct RestorePlan {
    segments: SegmentPlan,
    files: u64,
    bytes: u64,
    skipped: u64,
}

async fn plan_restore(
    entries: Vec<(u32, ManifestEntry)>,
    targets: Arc<RestoreTargets>,
    reporter: &ProgressReporter,
) -> flow_like_types::Result<RestorePlan> {
    reporter.phase(ArchivePhase::Planning, 0, entries.len() as u64);
    let needed: Vec<Option<(u32, String, String, u64)>> = stream::iter(entries)
        .map(|(segment, entry)| {
            let targets = targets.clone();
            async move {
                reporter.check()?;
                let target = targets.target(&entry.rel_path)?;
                let store = targets.store(entry.store);
                let outcome = if existing_matches(store, &target, entry.size, &entry.blake3).await?
                {
                    None
                } else {
                    Some((segment, entry.blake3, entry.rel_path, entry.size))
                };
                reporter.add_files(1);
                Ok::<_, flow_like_types::Error>(outcome)
            }
        })
        .buffer_unordered(HEAD_CONCURRENCY)
        .try_collect()
        .await?;

    let mut plan = RestorePlan::default();
    for outcome in needed {
        let Some((segment, blob, rel, size)) = outcome else {
            plan.skipped += 1;
            continue;
        };
        plan.files += 1;
        plan.bytes += size;
        plan.segments
            .entry(segment)
            .or_default()
            .entry(blob)
            .or_default()
            .push(rel);
    }
    Ok(plan)
}

async fn restore_from_zip<R: Read + Seek + Send + 'static>(
    mut archive: ZipArchive<R>,
    needed: HashMap<String, Vec<String>>,
    entries: Arc<HashMap<String, ManifestEntry>>,
    targets: Arc<RestoreTargets>,
    reporter: Reporter,
) -> flow_like_types::Result<()> {
    for (blob, rels) in needed {
        reporter.check()?;
        let size = rels
            .first()
            .and_then(|rel| entries.get(rel))
            .map(|e| e.size)
            .ok_or_else(|| anyhow!("Missing manifest entry for blob {blob}"))?;

        let (returned, bytes) = run_blocking({
            let blob = blob.clone();
            move || {
                let raw = read_zip_entry(&mut archive, &blob, size)?;
                if raw.len() as u64 != size || blake3_hex(&raw) != blob {
                    bail!("Integrity failure for blob {blob}");
                }
                Ok((archive, Bytes::from(raw)))
            }
        })
        .await?;
        archive = returned;

        stream::iter(rels)
            .map(|rel| {
                let bytes = bytes.clone();
                let entries = entries.clone();
                let targets = targets.clone();
                let reporter = reporter.clone();
                async move {
                    reporter.check()?;
                    let entry = entries
                        .get(&rel)
                        .ok_or_else(|| anyhow!("Missing manifest entry for {rel}"))?;
                    if entry.size != bytes.len() as u64 {
                        bail!("Integrity failure for {rel}");
                    }
                    let target = targets.target(&rel)?;
                    targets
                        .store(entry.store)
                        .put(&target, PutPayload::from_bytes(bytes))
                        .await
                        .map_err(|e| anyhow!("Failed to restore {rel}: {e}"))?;
                    reporter.add_bytes(entry.size);
                    reporter.add_files(1);
                    Ok::<(), flow_like_types::Error>(())
                }
            })
            .buffer_unordered(PUT_CONCURRENCY)
            .try_collect::<()>()
            .await?;
    }
    Ok(())
}

struct LocalApp {
    evict: HashSet<String>,
    visibility: Option<AppVisibility>,
}

async fn local_app_snapshot(app_state: &Arc<FlowLikeState>, app_id: &str) -> LocalApp {
    match App::load(app_id.to_string(), app_state.clone()).await {
        Ok(app) => LocalApp {
            evict: app.boards.into_iter().chain(app.templates).collect(),
            visibility: Some(app.visibility),
        },
        Err(_) => LocalApp {
            evict: HashSet::new(),
            visibility: None,
        },
    }
}

fn evict_boards(app_state: &FlowLikeState, ids: &HashSet<String>) {
    if ids.is_empty() {
        return;
    }
    let prefixes: Vec<String> = ids.iter().map(|id| format!("{id}-")).collect();
    app_state.board_registry().retain(|key, _| {
        !(ids.contains(key) || prefixes.iter().any(|prefix| key.starts_with(prefix)))
    });
}

async fn delete_ignore_missing(
    store: &Arc<dyn ObjectStore>,
    path: &Path,
) -> flow_like_types::Result<()> {
    match store.delete(path).await {
        Ok(()) | Err(flow_like_storage::object_store::Error::NotFound { .. }) => Ok(()),
        Err(e) => Err(anyhow!("Failed to delete {path}: {e}")),
    }
}

async fn remove_unlisted_files(
    targets: &Arc<RestoreTargets>,
    entries: &EntriesByRel,
    reporter: &ProgressReporter,
) -> flow_like_types::Result<u64> {
    let extra: Vec<Path> = list_app_files(&targets.meta, &targets.storage, &targets.base)
        .await?
        .into_iter()
        .filter(|f| !entries.contains_key(&f.rel))
        .map(|f| f.path)
        .collect();
    reporter.phase(ArchivePhase::Cleaning, 0, extra.len() as u64);
    let deleted = extra.len() as u64;
    stream::iter(extra)
        .map(|path| async move {
            reporter.check()?;
            delete_ignore_missing(&targets.meta, &path).await?;
            delete_ignore_missing(&targets.storage, &path).await?;
            reporter.add_files(1);
            Ok::<(), flow_like_types::Error>(())
        })
        .buffer_unordered(PUT_CONCURRENCY)
        .try_collect::<()>()
        .await?;
    Ok(deleted)
}

async fn finalize_import(
    app_state: Arc<FlowLikeState>,
    app_id: &str,
    local: LocalApp,
    reporter: &ProgressReporter,
) -> flow_like_types::Result<App> {
    reporter.phase(ArchivePhase::Finalizing, 0, 0);
    reporter.check()?;
    let LocalApp {
        mut evict,
        visibility,
    } = local;
    let mut app = App::load(app_id.to_string(), app_state.clone()).await?;
    evict.extend(app.boards.iter().cloned());
    evict.extend(app.templates.iter().cloned());
    evict_boards(&app_state, &evict);
    app.visibility = visibility.unwrap_or(AppVisibility::Offline);
    app.updated_at = SystemTime::now();
    app.save().await?;
    Ok(app)
}

struct ImportSession {
    app_state: Arc<FlowLikeState>,
    reporter: Reporter,
    mode: ImportMode,
    app_id: String,
    targets: Arc<RestoreTargets>,
    local: LocalApp,
    entries: EntriesByRel,
    plan: RestorePlan,
}

impl ImportSession {
    async fn open(
        app_state: Arc<FlowLikeState>,
        reporter: Reporter,
        mode: ImportMode,
        manifest: ExportManifest,
        legacy: bool,
    ) -> flow_like_types::Result<Self> {
        let app_id = manifest.app_id.clone();
        let targets = restore_targets(&app_state, &app_id).await?;
        let local = local_app_snapshot(&app_state, &app_id).await;
        let (planned, entries) = split_manifest(manifest, legacy);
        let plan = plan_restore(planned, targets.clone(), &reporter).await?;
        reporter.phase(ArchivePhase::Restoring, plan.bytes, plan.files);
        Ok(Self {
            app_state,
            reporter,
            mode,
            app_id,
            targets,
            local,
            entries,
            plan,
        })
    }

    fn take_segments(&mut self) -> SegmentPlan {
        std::mem::take(&mut self.plan.segments)
    }

    /// A failed restore leaves files half-written on purpose, but the
    /// registered boards must go so a later `App::save` cannot overwrite
    /// what was already restored with stale in-memory copies.
    async fn restore<F>(&self, restore: F) -> flow_like_types::Result<()>
    where
        F: std::future::Future<Output = flow_like_types::Result<()>>,
    {
        let result = restore.await;
        if result.is_err() {
            evict_boards(&self.app_state, &self.local.evict);
        }
        result
    }

    async fn finish(self) -> flow_like_types::Result<ImportReport> {
        let deleted_files = match self.mode {
            ImportMode::Merge => 0,
            ImportMode::Replace => {
                remove_unlisted_files(&self.targets, &self.entries, &self.reporter).await?
            }
        };
        let app = finalize_import(self.app_state, &self.app_id, self.local, &self.reporter).await?;
        Ok(ImportReport {
            app,
            mode: self.mode,
            restored_files: self.plan.files,
            skipped_files: self.plan.skipped,
            deleted_files,
        })
    }
}

async fn restore_targets(
    app_state: &Arc<FlowLikeState>,
    app_id: &str,
) -> flow_like_types::Result<Arc<RestoreTargets>> {
    let meta = FlowLikeState::project_meta_store(app_state)
        .await?
        .as_generic();
    let storage = FlowLikeState::project_storage_store(app_state)
        .await?
        .as_generic();
    Ok(Arc::new(RestoreTargets {
        meta,
        storage,
        base: app_base(app_id),
    }))
}

// ===== v2 import =====

async fn open_v2(source_file: PathBuf) -> flow_like_types::Result<(Arc<File>, ArchiveV2)> {
    run_blocking(move || {
        let mut file = File::open(&source_file)?;
        let layout = read_v2_layout(&mut file)?;
        Ok((Arc::new(file), layout))
    })
    .await
}

async fn unlock_v2(
    layout: &ArchiveV2,
    password: Option<Password>,
) -> flow_like_types::Result<Option<Arc<ArchiveKeys>>> {
    let Some(kdf) = layout.header.kdf else {
        return Ok(None);
    };
    let password = password.ok_or_else(|| anyhow!("Password required for encrypted archive"))?;
    let tag = layout
        .tag
        .ok_or_else(|| anyhow!("Archive is missing its tag"))?;
    let header_bytes = layout.header_bytes.clone();
    let index_bytes = layout.index_bytes.clone();
    let keys = run_blocking(move || {
        let keys = derive_archive_keys(&password, &kdf)?;
        let expected = binding_tag_v2(&keys.binding, &header_bytes, &index_bytes);
        if !constant_time_eq(&expected, &tag) {
            bail!(BINDING_TAG_ERROR);
        }
        Ok(keys)
    })
    .await?;
    Ok(Some(Arc::new(keys)))
}

async fn read_v2_manifest(
    source: Arc<File>,
    layout: &ArchiveV2,
    keys: Option<Arc<ArchiveKeys>>,
    reporter: Reporter,
) -> flow_like_types::Result<ExportManifest> {
    let chunk_size = layout.header.chunk_size;
    let manifest_entry = layout.segments[0];
    if manifest_entry.plain_len > MAX_MANIFEST_SEGMENT_LEN {
        bail!("Archive manifest segment exceeds {MAX_MANIFEST_SEGMENT_LEN} bytes");
    }
    run_blocking(move || {
        let cancelled = || reporter.cancelled();
        let reader = open_segment(
            &source,
            manifest_entry,
            0,
            keys.as_deref(),
            chunk_size,
            &cancelled,
        )?;
        let mut archive = ZipArchive::new(reader)?;
        parse_manifest(&mut archive, MAX_MANIFEST_SEGMENT_LEN)
    })
    .await
}

async fn import_v2(
    app_state: Arc<FlowLikeState>,
    source_file: PathBuf,
    password: Option<Password>,
    mode: ImportMode,
    reporter: Reporter,
) -> flow_like_types::Result<ImportReport> {
    let (source, layout) = open_v2(source_file).await?;
    let keys = unlock_v2(&layout, password).await?;
    let manifest =
        read_v2_manifest(source.clone(), &layout, keys.clone(), reporter.clone()).await?;

    let mut session =
        ImportSession::open(app_state, reporter.clone(), mode, manifest, false).await?;
    let entries = session.entries.clone();
    let targets = session.targets.clone();
    let chunk_size = layout.header.chunk_size;
    let segment_count = layout.segments.len() as u32;
    let workers = shard_count();
    // Decryption stages a whole plaintext segment on disk, so encrypted imports
    // cap how many segments are staged at once instead of scaling with workers.
    let staging = keys
        .is_some()
        .then(|| Arc::new(Semaphore::new(ENCRYPTED_SEGMENT_STAGING)));
    let segments = session.take_segments();
    let restore = stream::iter(segments)
        .map(|(segment, needed)| {
            let source = source.clone();
            let keys = keys.clone();
            let staging = staging.clone();
            let entries = entries.clone();
            let targets = targets.clone();
            let reporter = reporter.clone();
            let segment_entry =
                (segment > 0 && segment < segment_count).then(|| layout.segments[segment as usize]);
            async move {
                let _staging = match staging {
                    Some(staging) => Some(
                        staging
                            .acquire_owned()
                            .await
                            .map_err(|_| anyhow!("Segment staging budget closed"))?,
                    ),
                    None => None,
                };
                let segment_entry = segment_entry
                    .ok_or_else(|| anyhow!("Manifest references unknown segment {segment}"))?;
                if segment_entry.kind != SEGMENT_KIND_BLOBS {
                    bail!("Manifest references non-blob segment {segment}");
                }
                let archive = run_blocking({
                    let reporter = reporter.clone();
                    move || {
                        let cancelled = || reporter.cancelled();
                        let reader = open_segment(
                            &source,
                            segment_entry,
                            segment,
                            keys.as_deref(),
                            chunk_size,
                            &cancelled,
                        )?;
                        Ok(ZipArchive::new(reader)?)
                    }
                })
                .await?;
                restore_from_zip(archive, needed, entries, targets, reporter).await
            }
        })
        .buffer_unordered(workers)
        .try_collect::<()>();
    session.restore(restore).await?;

    session.finish().await
}

// ===== v1 import =====

fn derive_binding_key_v1(
    password: &str,
    salt: &[u8; BINDING_SALT_LEN],
) -> flow_like_types::Result<[u8; 32]> {
    argon2_derive(
        password.as_bytes(),
        salt,
        ARGON2_M_COST_KIB,
        ARGON2_T_COST,
        ARGON2_P_COST,
    )
}

fn compute_binding_tag_v1(
    key: &[u8; 32],
    trailer_bytes: &[u8; 24],
    manifest_ct: &[u8],
    prio_ct: &[u8],
    secondary_ct: &[u8],
) -> [u8; BINDING_TAG_LEN] {
    let mut hasher = blake3::Hasher::new_keyed(key);
    hasher.update(BINDING_CONTEXT_V1);
    hasher.update(trailer_bytes);
    hasher.update(manifest_ct);
    hasher.update(prio_ct);
    hasher.update(secondary_ct);
    *hasher.finalize().as_bytes()
}

fn decrypt_bytes_v1(password: &str, data: &[u8]) -> flow_like_types::Result<Vec<u8>> {
    use chacha20poly1305::{XNonce, aead::Aead};

    if data.len() < ENC_MAGIC.len() + SALT_LEN + XNONCE_LEN {
        return Err(anyhow!("Invalid encrypted archive"));
    }
    let (magic, rest) = data.split_at(ENC_MAGIC.len());
    if magic != ENC_MAGIC {
        return Err(anyhow!("Invalid encrypted archive header"));
    }
    let (salt, rest) = rest.split_at(SALT_LEN);
    let (nonce, ciphertext) = rest.split_at(XNONCE_LEN);

    let mut key = argon2_derive(
        password.as_bytes(),
        salt,
        ARGON2_M_COST_KIB,
        ARGON2_T_COST,
        ARGON2_P_COST,
    )?;
    let cipher = XChaCha20Poly1305::new_from_slice(&key).map_err(|e| anyhow!(e))?;
    key.zeroize();
    let nonce: [u8; XNONCE_LEN] = nonce
        .try_into()
        .map_err(|_| anyhow!("Invalid nonce length"))?;
    let nonce = XNonce::from(nonce);
    cipher
        .decrypt(&nonce, ciphertext)
        .map_err(|_| anyhow!("Decryption failed"))
}

#[derive(Clone, Copy)]
struct TrailerV1 {
    manifest_size: u64,
    prio_size: u64,
    secondary_size: u64,
}

impl TrailerV1 {
    fn to_bytes(self) -> [u8; 24] {
        let mut buf = [0u8; 24];
        buf[0..8].copy_from_slice(&self.manifest_size.to_le_bytes());
        buf[8..16].copy_from_slice(&self.prio_size.to_le_bytes());
        buf[16..24].copy_from_slice(&self.secondary_size.to_le_bytes());
        buf
    }

    fn from_bytes(bytes: &[u8; 24]) -> Self {
        Self {
            manifest_size: u64_at(bytes, 0),
            prio_size: u64_at(bytes, 8),
            secondary_size: u64_at(bytes, 16),
        }
    }

    fn segments_len(&self) -> flow_like_types::Result<u64> {
        self.manifest_size
            .checked_add(self.prio_size)
            .and_then(|n| n.checked_add(self.secondary_size))
            .ok_or_else(|| anyhow!("overflow"))
    }
}

enum LayoutV1 {
    Plain {
        manifest: Vec<u8>,
        prio: Vec<u8>,
        secondary: Vec<u8>,
    },
    Encrypted {
        trailer: TrailerV1,
        binding_salt: [u8; BINDING_SALT_LEN],
        binding_tag: [u8; BINDING_TAG_LEN],
        manifest_ct: Vec<u8>,
        prio_ct: Vec<u8>,
        secondary_ct: Vec<u8>,
    },
}

fn read_slice(f: &mut File, start: u64, len: u64) -> flow_like_types::Result<Vec<u8>> {
    f.seek(SeekFrom::Start(start))?;
    let mut buf = vec![0u8; len as usize];
    f.read_exact(&mut buf)?;
    Ok(buf)
}

fn read_v1_layout(mut f: File) -> flow_like_types::Result<LayoutV1> {
    const TAIL_LEN: u64 = (BINDING_SALT_LEN + BINDING_TAG_LEN) as u64;
    let total = f.metadata()?.len();
    if total < 24 {
        return Err(anyhow!("file too small"));
    }

    if total >= 24 + TAIL_LEN {
        let mut t_enc = [0u8; 24];
        f.seek(SeekFrom::End(-((24 + TAIL_LEN) as i64)))?;
        f.read_exact(&mut t_enc)?;
        let trailer = TrailerV1::from_bytes(&t_enc);
        let expected_total = trailer
            .segments_len()?
            .checked_add(24 + TAIL_LEN)
            .ok_or_else(|| anyhow!("overflow"))?;
        if expected_total == total {
            let m = trailer.manifest_size;
            let p = trailer.prio_size;
            let manifest_ct = read_slice(&mut f, 0, m)?;
            let prio_ct = read_slice(&mut f, m, p)?;
            let secondary_ct = read_slice(&mut f, m + p, trailer.secondary_size)?;
            f.seek(SeekFrom::End(-(TAIL_LEN as i64)))?;
            let mut binding_salt = [0u8; BINDING_SALT_LEN];
            let mut binding_tag = [0u8; BINDING_TAG_LEN];
            f.read_exact(&mut binding_salt)?;
            f.read_exact(&mut binding_tag)?;
            return Ok(LayoutV1::Encrypted {
                trailer,
                binding_salt,
                binding_tag,
                manifest_ct,
                prio_ct,
                secondary_ct,
            });
        }
    }

    let mut t_plain = [0u8; 24];
    f.seek(SeekFrom::End(-24))?;
    f.read_exact(&mut t_plain)?;
    let trailer = TrailerV1::from_bytes(&t_plain);
    let expected_total = trailer
        .segments_len()?
        .checked_add(24)
        .ok_or_else(|| anyhow!("overflow"))?;
    if expected_total != total {
        return Err(anyhow!("trailer sizes don't match file length"));
    }

    let m = trailer.manifest_size;
    let p = trailer.prio_size;
    let manifest = read_slice(&mut f, 0, m)?;
    let prio = read_slice(&mut f, m, p)?;
    let secondary = read_slice(&mut f, m + p, trailer.secondary_size)?;

    let looks_enc = |seg: &Vec<u8>| seg.starts_with(ENC_MAGIC);
    if looks_enc(&manifest) || looks_enc(&prio) || looks_enc(&secondary) {
        return Err(anyhow!(
            "Encrypted archive missing binding tag (unsupported legacy format)"
        ));
    }

    Ok(LayoutV1::Plain {
        manifest,
        prio,
        secondary,
    })
}

/// Manifest zip plus the (still sealed when encrypted) blob segments of a v1 archive.
struct UnlockedV1 {
    manifest: ExportManifest,
    prio: Vec<u8>,
    secondary: Vec<u8>,
    password: Option<Password>,
}

async fn unlock_v1(
    layout: LayoutV1,
    password: Option<Password>,
) -> flow_like_types::Result<UnlockedV1> {
    let (manifest_zip, prio, secondary, password) = match layout {
        LayoutV1::Plain {
            manifest,
            prio,
            secondary,
        } => (manifest, prio, secondary, None),
        LayoutV1::Encrypted {
            trailer,
            binding_salt,
            binding_tag,
            manifest_ct,
            prio_ct,
            secondary_ct,
        } => {
            let password =
                password.ok_or_else(|| anyhow!("Password required for encrypted archive"))?;
            let (manifest_zip, prio_ct, secondary_ct) = run_blocking({
                let password = password.clone();
                move || {
                    let binding_key = derive_binding_key_v1(&password, &binding_salt)?;
                    let expected = compute_binding_tag_v1(
                        &binding_key,
                        &trailer.to_bytes(),
                        &manifest_ct,
                        &prio_ct,
                        &secondary_ct,
                    );
                    if !constant_time_eq(&expected, &binding_tag) {
                        bail!(BINDING_TAG_ERROR);
                    }
                    let manifest_zip = decrypt_bytes_v1(&password, &manifest_ct)?;
                    Ok((manifest_zip, prio_ct, secondary_ct))
                }
            })
            .await?;
            (manifest_zip, prio_ct, secondary_ct, Some(password))
        }
    };

    let manifest = run_blocking(move || {
        let mut archive = ZipArchive::new(Cursor::new(manifest_zip))?;
        parse_manifest(&mut archive, MAX_MANIFEST_SEGMENT_LEN)
    })
    .await?;

    Ok(UnlockedV1 {
        manifest,
        prio,
        secondary,
        password,
    })
}

async fn import_v1(
    app_state: Arc<FlowLikeState>,
    source_file: PathBuf,
    password: Option<Password>,
    mode: ImportMode,
    reporter: Reporter,
) -> flow_like_types::Result<ImportReport> {
    let layout = run_blocking(move || read_v1_layout(File::open(&source_file)?)).await?;
    let unlocked = unlock_v1(layout, password).await?;

    let mut session =
        ImportSession::open(app_state, reporter.clone(), mode, unlocked.manifest, true).await?;
    let mut segments = session.take_segments();
    let entries = session.entries.clone();
    let targets = session.targets.clone();
    let raws = [(1u32, unlocked.prio), (2u32, unlocked.secondary)];
    let password = unlocked.password;
    let restore = async {
        for (segment, raw) in raws {
            let Some(needed) = segments.remove(&segment) else {
                continue;
            };
            reporter.check()?;
            let archive = run_blocking({
                let password = password.clone();
                move || {
                    let plain = match password {
                        Some(password) => decrypt_bytes_v1(&password, &raw)?,
                        None => raw,
                    };
                    Ok(ZipArchive::new(Cursor::new(plain))?)
                }
            })
            .await?;
            restore_from_zip(
                archive,
                needed,
                entries.clone(),
                targets.clone(),
                reporter.clone(),
            )
            .await?;
        }
        Ok::<(), flow_like_types::Error>(())
    };
    session.restore(restore).await?;

    session.finish().await
}

// ===== Public API =====

impl App {
    pub async fn export_archive(
        &self,
        password: Option<String>,
        target_file: PathBuf,
    ) -> flow_like_types::Result<PathBuf> {
        let report = self
            .export_archive_with(
                ExportOptions {
                    password,
                    compact_tables: false,
                },
                target_file,
                ArchiveObserver::default(),
            )
            .await?;
        Ok(report.path)
    }

    pub async fn export_archive_with(
        &self,
        options: ExportOptions,
        mut target_file: PathBuf,
        observer: ArchiveObserver,
    ) -> flow_like_types::Result<ExportReport> {
        let reporter = ProgressReporter::new(observer);
        let password: Option<Password> = options.password.map(|pw| Arc::new(Zeroizing::new(pw)));
        apply_archive_suffix(&mut target_file, password.is_some());

        match self
            .export_inner(
                password,
                options.compact_tables,
                target_file.clone(),
                reporter.clone(),
            )
            .await
        {
            Ok(report) => {
                reporter.finish();
                Ok(report)
            }
            Err(e) if is_cancelled_error(&e) => Err(anyhow!(ARCHIVE_CANCELLED)),
            Err(e) => Err(e),
        }
    }

    async fn export_inner(
        &self,
        password: Option<Password>,
        compact_tables: bool,
        target_file: PathBuf,
        reporter: Reporter,
    ) -> flow_like_types::Result<ExportReport> {
        reporter.phase(ArchivePhase::Listing, 0, 0);
        reporter.check()?;
        let app_state = self
            .app_state
            .clone()
            .ok_or(anyhow!("App state not found"))?;
        let meta = FlowLikeState::project_meta_store(&app_state)
            .await?
            .as_generic();
        let storage = FlowLikeState::project_storage_store(&app_state)
            .await?
            .as_generic();
        let base = app_base(&self.id);

        let mut files = list_app_files(&meta, &storage, &base).await?;
        let compaction = if compact_tables && !table_stats(&files).is_empty() {
            reporter.check()?;
            let report = self.compact_tables_observed(&reporter).await?;
            reporter.phase(ArchivePhase::Listing, 0, 0);
            files = list_app_files(&meta, &storage, &base).await?;
            Some(report)
        } else {
            None
        };

        if let Some(parent) = target_file.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }

        let shards = shard_count();
        let collected =
            collect_blobs(&self.id, meta, storage, files, shards, reporter.clone()).await?;
        let file_count =
            (collected.manifest.prio.len() + collected.manifest.secondary.len()) as u64;

        let manifest_zip = {
            let manifest = collected.manifest;
            run_blocking(move || write_manifest_zip(&manifest)).await?
        };

        let mut sources = Vec::with_capacity(collected.shards.len() + 1);
        sources.push(SegmentSource {
            kind: SEGMENT_KIND_MANIFEST,
            plain_len: manifest_zip.metadata()?.len(),
            file: manifest_zip,
        });
        for file in collected.shards {
            sources.push(SegmentSource {
                kind: SEGMENT_KIND_BLOBS,
                plain_len: file.metadata()?.len(),
                file,
            });
        }

        let (header, keys) = match password {
            Some(password) => {
                let mut salt = [0u8; KDF_SALT_LEN];
                OsRng.try_fill_bytes(&mut salt)?;
                let kdf = KdfParams {
                    salt,
                    m_cost: ARGON2_M_COST_KIB,
                    t_cost: ARGON2_T_COST,
                    p_cost: ARGON2_P_COST,
                };
                let keys = run_blocking(move || derive_archive_keys(&password, &kdf)).await?;
                (
                    HeaderV2 {
                        chunk_size: CHUNK_SIZE,
                        kdf: Some(kdf),
                    },
                    Some(Arc::new(keys)),
                )
            }
            None => (
                HeaderV2 {
                    chunk_size: CHUNK_SIZE,
                    kdf: None,
                },
                None,
            ),
        };

        let bytes_written =
            assemble_archive(target_file.clone(), header, sources, keys, shards, reporter).await?;

        Ok(ExportReport {
            path: target_file,
            bytes_written,
            file_count,
            blob_count: collected.blob_count,
            compaction,
        })
    }

    pub async fn export_preflight(&self) -> flow_like_types::Result<ExportPreflight> {
        let app_state = self
            .app_state
            .clone()
            .ok_or(anyhow!("App state not found"))?;
        let meta = FlowLikeState::project_meta_store(&app_state)
            .await?
            .as_generic();
        let storage = FlowLikeState::project_storage_store(&app_state)
            .await?
            .as_generic();
        let files = list_app_files(&meta, &storage, &app_base(&self.id)).await?;
        let tables = table_stats(&files);

        let mut secret_variables = Vec::new();
        for board_id in &self.boards {
            let board = match self.open_board(board_id.clone(), Some(false), None).await {
                Ok(board) => board,
                Err(e) => {
                    tracing::warn!(board_id, error = %e, "export preflight could not open board");
                    continue;
                }
            };
            let board = board.lock().await;
            for variable in board.variables.values() {
                if variable.secret && secret_has_value(variable.default_value.as_deref()) {
                    secret_variables.push(SecretVariableRef {
                        board_id: board.id.clone(),
                        board_name: board.name.clone(),
                        variable_id: variable.id.clone(),
                        variable_name: variable.name.clone(),
                    });
                }
            }
        }

        #[cfg(feature = "flow-runtime")]
        let compaction_available = app_state
            .config
            .read()
            .await
            .callbacks
            .build_project_database
            .is_some();
        #[cfg(not(feature = "flow-runtime"))]
        let compaction_available = false;

        Ok(ExportPreflight {
            total_bytes: files.iter().map(|f| f.size).sum(),
            file_count: files.len() as u64,
            secret_variables,
            reclaimable_bytes: tables.iter().map(|t| t.history_bytes).sum(),
            tables,
            compaction_available,
        })
    }

    pub async fn compact_tables(&self) -> flow_like_types::Result<CompactionReport> {
        self.compact_tables_observed(&ProgressReporter::new(ArchiveObserver::default()))
            .await
    }

    #[cfg(feature = "flow-runtime")]
    async fn compact_tables_observed(
        &self,
        reporter: &ProgressReporter,
    ) -> flow_like_types::Result<CompactionReport> {
        use flow_like_storage::databases::vector::{VectorStore, lancedb::LanceDBVectorStore};

        let app_state = self
            .app_state
            .clone()
            .ok_or(anyhow!("App state not found"))?;
        let (build_project_database, write_options) = {
            let config = app_state.config.read().await;
            (
                config.callbacks.build_project_database.clone(),
                config.callbacks.lance_write_options.clone(),
            )
        };
        let build_project_database = build_project_database
            .ok_or_else(|| anyhow!("No project database builder registered"))?;

        let base = app_base(&self.id);
        let db_path = base.clone().join("storage").join("db");
        let connection = app_state
            .with_lance_session(build_project_database(db_path.clone()))
            .execute()
            .await
            .map_err(|e| anyhow!("Failed to open project database {db_path}: {e}"))?;
        let names = connection
            .table_names()
            .execute()
            .await
            .map_err(|e| anyhow!("Failed to list tables in {db_path}: {e}"))?;
        let storage = FlowLikeState::project_storage_store(&app_state)
            .await?
            .as_generic();

        let mut report = CompactionReport {
            tables: Vec::with_capacity(names.len()),
            bytes_before: 0,
            bytes_after: 0,
        };
        reporter.phase(ArchivePhase::Compacting, 0, names.len() as u64);
        for name in names {
            reporter.check()?;
            let prefix = db_path.clone().join(format!("{name}.lance"));
            let bytes_before = prefix_bytes(&storage, &prefix).await.unwrap_or(0);
            let mut store =
                LanceDBVectorStore::from_connection(connection.clone(), name.clone()).await;
            if let Some(options) = write_options.clone() {
                store.set_write_options(options);
            }
            // Export compaction must preserve snapshots used by training runs and branches.
            let result = store.optimize(true).await;
            let bytes_after = prefix_bytes(&storage, &prefix)
                .await
                .unwrap_or(bytes_before);
            if let Err(e) = &result {
                tracing::warn!(table = %name, error = %e, "table compaction failed");
            }
            report.bytes_before += bytes_before;
            report.bytes_after += bytes_after;
            report.tables.push(TableCompaction {
                name,
                ok: result.is_ok(),
                error: result.err().map(|e| e.to_string()),
                bytes_before,
                bytes_after,
            });
            reporter.add_files(1);
        }
        Ok(report)
    }

    #[cfg(not(feature = "flow-runtime"))]
    async fn compact_tables_observed(
        &self,
        _reporter: &ProgressReporter,
    ) -> flow_like_types::Result<CompactionReport> {
        Err(anyhow!(
            "table compaction requires the flow-runtime feature"
        ))
    }

    pub async fn import_archive(
        app_state: Arc<FlowLikeState>,
        source_file: PathBuf,
        password: Option<String>,
    ) -> flow_like_types::Result<Self> {
        let report = Self::import_archive_with(
            app_state,
            source_file,
            ImportOptions {
                password,
                mode: ImportMode::Merge,
            },
            ArchiveObserver::default(),
        )
        .await?;
        Ok(report.app)
    }

    pub async fn import_archive_with(
        app_state: Arc<FlowLikeState>,
        source_file: PathBuf,
        options: ImportOptions,
        observer: ArchiveObserver,
    ) -> flow_like_types::Result<ImportReport> {
        let reporter = ProgressReporter::new(observer);
        let password: Option<Password> = options.password.map(|pw| Arc::new(Zeroizing::new(pw)));
        reporter.phase(ArchivePhase::Reading, 0, 0);
        let result = async {
            reporter.check()?;
            if is_v2_file(source_file.clone()).await? {
                import_v2(
                    app_state,
                    source_file,
                    password,
                    options.mode,
                    reporter.clone(),
                )
                .await
            } else {
                import_v1(
                    app_state,
                    source_file,
                    password,
                    options.mode,
                    reporter.clone(),
                )
                .await
            }
        }
        .await;
        match result {
            Ok(report) => {
                reporter.finish();
                Ok(report)
            }
            Err(e) if is_cancelled_error(&e) => Err(anyhow!(ARCHIVE_CANCELLED)),
            Err(e) => Err(e),
        }
    }

    pub async fn inspect_archive(
        app_state: Arc<FlowLikeState>,
        source_file: PathBuf,
        password: Option<String>,
    ) -> flow_like_types::Result<ArchiveInfo> {
        let password: Option<Password> = password.map(|pw| Arc::new(Zeroizing::new(pw)));
        let mut info = if is_v2_file(source_file.clone()).await? {
            let (source, layout) = open_v2(source_file).await?;
            let encrypted = layout.header.encrypted();
            let mut info = ArchiveInfo::sealed(2, encrypted);
            if !encrypted || password.is_some() {
                let keys = unlock_v2(&layout, password).await?;
                let reporter = ProgressReporter::new(ArchiveObserver::default());
                info.describe(&read_v2_manifest(source, &layout, keys, reporter).await?);
            }
            info
        } else {
            let layout = run_blocking(move || read_v1_layout(File::open(&source_file)?)).await?;
            let encrypted = matches!(layout, LayoutV1::Encrypted { .. });
            let mut info = ArchiveInfo::sealed(1, encrypted);
            if !encrypted || password.is_some() {
                info.describe(&unlock_v1(layout, password).await?.manifest);
            }
            info
        };

        if let Some(app_id) = &info.app_id
            && let Ok(app) = App::load(app_id.clone(), app_state).await
        {
            info.exists_locally = true;
            info.local_visibility = Some(app.visibility);
        }
        Ok(info)
    }
}

impl ArchiveInfo {
    fn sealed(format_version: u32, encrypted: bool) -> Self {
        Self {
            format_version,
            encrypted,
            app_id: None,
            created_at: None,
            file_count: None,
            total_bytes: None,
            exists_locally: false,
            local_visibility: None,
        }
    }

    fn describe(&mut self, manifest: &ExportManifest) {
        let entries = manifest.prio.values().chain(manifest.secondary.values());
        self.app_id = Some(manifest.app_id.clone());
        self.created_at = Some(manifest.created_at);
        self.file_count = Some((manifest.prio.len() + manifest.secondary.len()) as u64);
        self.total_bytes = Some(entries.map(|e| e.size).sum());
    }
}

async fn is_v2_file(source_file: PathBuf) -> flow_like_types::Result<bool> {
    run_blocking(move || {
        let mut file = File::open(&source_file)?;
        let mut magic = [0u8; 8];
        Ok(file.read_exact(&mut magic).is_ok() && &magic == MAGIC_V2)
    })
    .await
}

#[cfg(test)]
mod legacy_v1 {
    use super::*;

    pub(super) enum ZipWriteCmd {
        File { name: String, data: Vec<u8> },
        Done,
    }

    pub(super) fn zip_write_streaming(
        mut file: File,
        rx: std::sync::mpsc::Receiver<ZipWriteCmd>,
    ) -> flow_like_types::Result<()> {
        let mut zw = ZipWriter::new(&mut file);
        let opts = FileOptions::<ExtendedFileOptions>::default()
            .compression_method(CompressionMethod::Deflated)
            .large_file(true);

        while let Ok(cmd) = rx.recv() {
            match cmd {
                ZipWriteCmd::File { name, data } => {
                    zw.start_file(name, opts.clone())?;
                    zw.write_all(&data)?;
                }
                ZipWriteCmd::Done => break,
            }
        }

        zw.finish()?;
        file.flush()?;
        Ok(())
    }

    pub(super) fn encrypt_bytes(password: &str, plain: &[u8]) -> flow_like_types::Result<Vec<u8>> {
        use chacha20poly1305::{XNonce, aead::Aead};

        let mut salt = [0u8; SALT_LEN];
        let mut nonce = [0u8; XNONCE_LEN];
        OsRng.try_fill_bytes(&mut salt)?;
        OsRng.try_fill_bytes(&mut nonce)?;

        let mut key = argon2_derive(
            password.as_bytes(),
            &salt,
            ARGON2_M_COST_KIB,
            ARGON2_T_COST,
            ARGON2_P_COST,
        )?;
        let cipher = XChaCha20Poly1305::new_from_slice(&key).map_err(|e| anyhow!(e))?;
        key.zeroize();
        let ciphertext = cipher
            .encrypt(&XNonce::from(nonce), plain)
            .map_err(|e| anyhow!(e))?;

        let mut out =
            Vec::with_capacity(ENC_MAGIC.len() + SALT_LEN + XNONCE_LEN + ciphertext.len());
        out.extend_from_slice(ENC_MAGIC);
        out.extend_from_slice(&salt);
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ciphertext);
        Ok(out)
    }

    pub(super) async fn export_archive(
        app: &App,
        password: Option<String>,
        mut target_file: PathBuf,
    ) -> flow_like_types::Result<PathBuf> {
        apply_archive_suffix(&mut target_file, password.is_some());

        let app_state = app
            .app_state
            .clone()
            .ok_or(anyhow!("App state not found"))?;
        let meta = FlowLikeState::project_meta_store(&app_state)
            .await?
            .as_generic();
        let storage = FlowLikeState::project_storage_store(&app_state)
            .await?
            .as_generic();

        let base = app_base(&app.id);
        let base_prefix = format!("{base}/");
        let mut manifest = ExportManifest {
            version: 1,
            app_id: app.id.clone(),
            created_at: now_unix(),
            prio: HashMap::new(),
            secondary: HashMap::new(),
        };

        let (prio_tx, prio_rx) = std::sync::mpsc::channel::<ZipWriteCmd>();
        let (secondary_tx, secondary_rx) = std::sync::mpsc::channel::<ZipWriteCmd>();
        let (manifest_tx, manifest_rx) = std::sync::mpsc::channel::<ZipWriteCmd>();
        let prio_zip = tempfile()?;
        let mut prio_zip_clone = prio_zip.try_clone()?;
        let secondary_zip = tempfile()?;
        let mut secondary_zip_clone = secondary_zip.try_clone()?;
        let manifest_file = tempfile()?;
        let mut manifest_file_clone = manifest_file.try_clone()?;

        let prio_handle = task::spawn_blocking(move || zip_write_streaming(prio_zip, prio_rx));
        let secondary_handle =
            task::spawn_blocking(move || zip_write_streaming(secondary_zip, secondary_rx));
        let manifest_handle =
            task::spawn_blocking(move || zip_write_streaming(manifest_file, manifest_rx));

        let mut prio_dedup = HashSet::new();
        let mut secondary_dedup = HashSet::new();

        for (kind, store) in [(StoreKind::Meta, &meta), (StoreKind::Storage, &storage)] {
            for (path, size) in list_store_files(store, &base).await? {
                let full = path.to_string();
                let rel = full.strip_prefix(&base_prefix).unwrap_or(&full).to_string();
                let bytes = store.get(&path).await?.bytes().await?.to_vec();
                let hash = blake3_hex(&bytes);
                let classification = classify_path(&rel);
                let (tx, dedup, map) = match classification {
                    PathKind::Prio => (&prio_tx, &mut prio_dedup, &mut manifest.prio),
                    PathKind::Secondary => {
                        (&secondary_tx, &mut secondary_dedup, &mut manifest.secondary)
                    }
                };
                if dedup.insert(hash.clone()) {
                    tx.send(ZipWriteCmd::File {
                        name: hash.clone(),
                        data: bytes,
                    })
                    .map_err(|e| anyhow!("ZIP writer channel closed: {}", e))?;
                }
                map.insert(
                    rel.clone(),
                    ManifestEntry {
                        store: kind,
                        rel_path: rel,
                        size,
                        blake3: hash,
                        segment: 0,
                    },
                );
            }
        }

        prio_tx.send(ZipWriteCmd::Done).ok();
        secondary_tx.send(ZipWriteCmd::Done).ok();
        prio_handle.await??;
        secondary_handle.await??;

        manifest_tx
            .send(ZipWriteCmd::File {
                name: MANIFEST_ENTRY_NAME.to_string(),
                data: flow_like_types::json::to_vec(&manifest)?,
            })
            .map_err(|e| anyhow!("ZIP writer channel closed: {}", e))?;
        manifest_tx.send(ZipWriteCmd::Done).ok();
        manifest_handle.await??;

        let mut trailer = TrailerV1 {
            manifest_size: 0,
            prio_size: 0,
            secondary_size: 0,
        };
        prio_zip_clone.seek(SeekFrom::Start(0))?;
        secondary_zip_clone.seek(SeekFrom::Start(0))?;
        manifest_file_clone.seek(SeekFrom::Start(0))?;
        let mut manifest_plain = Vec::new();
        manifest_file_clone.read_to_end(&mut manifest_plain)?;
        let mut prio_plain = Vec::new();
        prio_zip_clone.read_to_end(&mut prio_plain)?;
        let mut secondary_plain = Vec::new();
        secondary_zip_clone.read_to_end(&mut secondary_plain)?;

        let mut file = File::create(&target_file)?;
        if let Some(pw) = password {
            let enc_manifest = encrypt_bytes(&pw, &manifest_plain)?;
            let enc_prio = encrypt_bytes(&pw, &prio_plain)?;
            let enc_secondary = encrypt_bytes(&pw, &secondary_plain)?;
            trailer.manifest_size = enc_manifest.len() as u64;
            trailer.prio_size = enc_prio.len() as u64;
            trailer.secondary_size = enc_secondary.len() as u64;
            let trailer_bytes = trailer.to_bytes();

            let mut binding_salt = [0u8; BINDING_SALT_LEN];
            OsRng.try_fill_bytes(&mut binding_salt)?;
            let binding_key = derive_binding_key_v1(&pw, &binding_salt)?;
            let binding_tag = compute_binding_tag_v1(
                &binding_key,
                &trailer_bytes,
                &enc_manifest,
                &enc_prio,
                &enc_secondary,
            );

            file.write_all(&enc_manifest)?;
            file.write_all(&enc_prio)?;
            file.write_all(&enc_secondary)?;
            file.write_all(&trailer_bytes)?;
            file.write_all(&binding_salt)?;
            file.write_all(&binding_tag)?;
        } else {
            trailer.manifest_size = manifest_plain.len() as u64;
            trailer.prio_size = prio_plain.len() as u64;
            trailer.secondary_size = secondary_plain.len() as u64;
            file.write_all(&manifest_plain)?;
            file.write_all(&prio_plain)?;
            file.write_all(&secondary_plain)?;
            file.write_all(&trailer.to_bytes())?;
        }
        file.flush()?;
        Ok(target_file)
    }
}

#[cfg(test)]
mod tests {
    use super::legacy_v1::{ZipWriteCmd, encrypt_bytes, zip_write_streaming};
    use super::*;
    use crate::{
        app::{AppExecutionMode, AppStatus, AppVisibility},
        flow::{
            board::Board,
            pin::ValueType,
            variable::{Variable, VariableType},
        },
        state::FlowLikeConfig,
        utils::http::HTTPClient,
    };
    use flow_like_storage::{
        display_file_name,
        files::store::FlowLikeStore,
        object_store::{PutPayload, memory::InMemory, path::Path as ObjectPath},
    };
    use flow_like_types::{rand::RngCore, sync::Mutex, tokio};
    use std::{
        fs::File,
        io::{Read, Write},
        sync::Arc,
        time::SystemTime,
    };
    use tempfile::NamedTempFile;

    fn setup_state() -> (Arc<FlowLikeState>, Arc<InMemory>, Arc<InMemory>) {
        let mut config = FlowLikeConfig::new();
        let meta_store = Arc::new(InMemory::new());
        let storage_store = Arc::new(InMemory::new());
        config.register_app_meta_store(FlowLikeStore::Memory(meta_store.clone()));
        config.register_app_storage_store(FlowLikeStore::Memory(storage_store.clone()));
        let http_client = HTTPClient::new_without_refetch();
        let state = FlowLikeState::new(config, http_client);
        (Arc::new(state), meta_store, storage_store)
    }

    fn make_app(id: &str, state: Arc<FlowLikeState>) -> App {
        App {
            id: id.to_string(),
            app_type: None,
            status: AppStatus::Active,
            visibility: AppVisibility::Private,
            authors: vec![],
            bits: vec![],
            boards: vec![],
            events: vec![],
            templates: vec![],
            changelog: None,
            primary_category: None,
            secondary_category: None,
            rating_sum: 0,
            rating_count: 0,
            download_count: 0,
            interactions_count: 0,
            avg_rating: None,
            relevance_score: None,
            execution_mode: AppExecutionMode::Any,
            updated_at: SystemTime::now(),
            created_at: SystemTime::now(),
            version: None,
            frontend: None,
            price: None,
            app_state: Some(state),
            widget_ids: vec![],
            page_ids: vec![],
            packages: HashMap::new(),
            allow_forking: false,
            forked_at: None,
            forked_from: None,
        }
    }

    async fn seed_sample_objects(meta: &Arc<InMemory>, storage: &Arc<InMemory>, app_id: &str) {
        let meta_path = ObjectPath::from("apps").join(app_id).join("config.meta");
        meta.put(
            &meta_path,
            PutPayload::from_bytes(Bytes::from_static(b"meta config")),
        )
        .await
        .expect("seed meta");

        let storage_path = ObjectPath::from("apps").join(app_id).join("data.bin");
        storage
            .put(
                &storage_path,
                PutPayload::from_bytes(Bytes::from_static(b"storage data")),
            )
            .await
            .expect("seed blob");
    }

    fn random_bytes(len: usize, seed: u64) -> Bytes {
        use flow_like_types::rand::{SeedableRng, rngs::StdRng};
        let mut data = vec![0u8; len];
        StdRng::seed_from_u64(seed).fill_bytes(&mut data);
        Bytes::from(data)
    }

    fn encoded_fixture(app_id: &str) -> Vec<(ObjectPath, Bytes)> {
        let base = ObjectPath::from("apps").join(app_id);
        let shared = Bytes::from_static(b"identical content stored once");
        vec![
            (
                base.clone().join("upload").join("Bericht [2024].pdf"),
                Bytes::from_static(b"%PDF-1.7 bericht"),
            ),
            (
                base.clone().join("upload").join("ümlaut ä.csv"),
                Bytes::from_static(b"a;b\n1;2\n"),
            ),
            (
                base.clone().join("upload").join("Übersicht (2)#1.pdf"),
                Bytes::from_static(b"%PDF-1.7 uebersicht"),
            ),
            (
                base.clone().join("notes").join("a#1.md"),
                Bytes::from_static(b"# note one"),
            ),
            (
                base.clone()
                    .join("nested")
                    .join("dir")
                    .join("deeper")
                    .join("file.txt"),
                Bytes::from_static(b"deeply nested"),
            ),
            (base.clone().join("dup").join("a.txt"), shared.clone()),
            (base.clone().join("dup").join("b.txt"), shared),
            (
                base.join("upload").join("big.bin"),
                random_bytes(6 * 1024 * 1024 + 4321, 7),
            ),
        ]
    }

    async fn seed_encoded_fixture(
        storage: &Arc<InMemory>,
        app_id: &str,
    ) -> Vec<(ObjectPath, Bytes)> {
        let fixture = encoded_fixture(app_id);
        for (path, bytes) in &fixture {
            storage
                .put(path, PutPayload::from_bytes(bytes.clone()))
                .await
                .expect("seed fixture");
        }
        fixture
    }

    async fn list_keys(store: &Arc<InMemory>, app_id: &str) -> Vec<String> {
        let store: Arc<dyn ObjectStore> = store.clone();
        let mut keys: Vec<String> = list_store_files(&store, &app_base(app_id))
            .await
            .expect("list")
            .into_iter()
            .map(|(p, _)| p.to_string())
            .collect();
        keys.sort();
        keys
    }

    async fn get_bytes(store: &Arc<InMemory>, path: &ObjectPath) -> Bytes {
        store
            .get(path)
            .await
            .expect("object present")
            .bytes()
            .await
            .expect("object bytes")
    }

    async fn assert_roundtrip(password: Option<&str>, app_id: &str) {
        let (state, meta_store, storage_store) = setup_state();
        let app = make_app(app_id, state.clone());
        app.save().await.expect("seed manifest");
        seed_sample_objects(&meta_store, &storage_store, app_id).await;
        let fixture = seed_encoded_fixture(&storage_store, app_id).await;
        let expected_keys = list_keys(&storage_store, app_id).await;

        let tempdir = tempfile::tempdir().expect("tmpdir");
        let report = app
            .export_archive_with(
                ExportOptions {
                    password: password.map(str::to_string),
                    compact_tables: false,
                },
                tempdir.path().join("archive"),
                ArchiveObserver::default(),
            )
            .await
            .expect("export");
        assert_eq!(report.file_count, fixture.len() as u64 + 3);
        assert_eq!(report.blob_count, fixture.len() as u64 + 2);
        assert_eq!(
            report.bytes_written,
            std::fs::metadata(&report.path).expect("archive").len()
        );
        let mut magic = [0u8; 8];
        File::open(&report.path)
            .unwrap()
            .read_exact(&mut magic)
            .unwrap();
        assert_eq!(&magic, MAGIC_V2);

        let (import_state, import_meta, import_storage) = setup_state();
        let imported = App::import_archive(
            import_state,
            report.path.clone(),
            password.map(str::to_string),
        )
        .await
        .expect("import");
        assert_eq!(imported.id, app_id);

        assert_eq!(list_keys(&import_storage, app_id).await, expected_keys);
        for (path, bytes) in &fixture {
            assert_eq!(&get_bytes(&import_storage, path).await, bytes, "{path}");
        }
        let config_path = ObjectPath::from("apps").join(app_id).join("config.meta");
        assert_eq!(
            get_bytes(&import_meta, &config_path).await,
            Bytes::from_static(b"meta config")
        );
        let manifest_path = ObjectPath::from("apps").join(app_id).join("manifest.app");
        assert!(!get_bytes(&import_meta, &manifest_path).await.is_empty());
    }

    // ---------- Legacy (v1) encryption primitives ----------

    fn is_zip(data: &[u8]) -> bool {
        data.len() >= 4
            && (&data[..4] == b"PK\x03\x04"
                || &data[..4] == b"PK\x05\x06"
                || &data[..4] == b"PK\x06\x06")
    }

    #[test]
    fn encrypt_then_decrypt_roundtrip() {
        let pw = "s3cret!";
        let plain = b"hello flow-like";

        let enc = encrypt_bytes(pw, plain).expect("encrypt");
        assert_ne!(enc, plain);
        assert!(enc.starts_with(ENC_MAGIC));
        assert!(enc.len() > ENC_MAGIC.len() + SALT_LEN + XNONCE_LEN);

        let enc2 = encrypt_bytes(pw, plain).expect("encrypt");
        assert_ne!(enc, enc2, "randomized encryption should differ per call");

        let dec = decrypt_bytes_v1(pw, &enc).expect("decrypt");
        assert_eq!(dec, plain);
    }

    #[test]
    fn decrypt_with_wrong_password_fails() {
        let enc = encrypt_bytes("pw1", b"top secret").expect("encrypt");
        let err = decrypt_bytes_v1("pw2", &enc).unwrap_err();
        let msg = format!("{err:#}").to_lowercase();
        assert!(
            msg.contains("decrypt") || msg.contains("failed"),
            "got: {msg}"
        );
    }

    #[test]
    fn decrypt_with_wrong_magic_fails() {
        let mut bad = encrypt_bytes("pw", b"abc").expect("encrypt");
        bad[0] ^= 0xFF;
        let err = decrypt_bytes_v1("pw", &bad).unwrap_err();
        assert!(
            format!("{err:#}")
                .to_lowercase()
                .contains("invalid encrypted archive header")
        );
    }

    #[test]
    fn encryption_output_is_not_zip() {
        let enc = encrypt_bytes("pw", b"some payload").expect("encrypt");
        assert!(!is_zip(&enc));
    }

    // ---------- v2 primitives ----------

    fn test_keys() -> ArchiveKeys {
        ArchiveKeys {
            segment: [7u8; 32],
            binding: [9u8; 32],
        }
    }

    fn stream_roundtrip(plain: &[u8], chunk_size: u32) {
        let keys = test_keys();
        let mut stored = Vec::new();
        encrypt_segment(
            &mut Cursor::new(plain),
            plain.len() as u64,
            &keys.segment,
            3,
            chunk_size,
            &|| false,
            &mut stored,
        )
        .expect("encrypt");
        assert_eq!(
            stored.len() as u64,
            stored_len_for(plain.len() as u64, chunk_size)
        );
        assert!(!is_zip(&stored));

        let mut out = Vec::new();
        let written = decrypt_segment(
            &mut Cursor::new(&stored),
            stored.len() as u64,
            &keys.segment,
            3,
            chunk_size,
            &|| false,
            &mut out,
        )
        .expect("decrypt");
        assert_eq!(written, plain.len() as u64);
        assert_eq!(out, plain);

        let mut wrong_segment = Vec::new();
        assert!(
            decrypt_segment(
                &mut Cursor::new(&stored),
                stored.len() as u64,
                &keys.segment,
                4,
                chunk_size,
                &|| false,
                &mut wrong_segment,
            )
            .is_err(),
            "segment index is bound through the AAD"
        );
    }

    #[test]
    fn stream_encryption_roundtrips_every_chunk_shape() {
        stream_roundtrip(b"", 16);
        stream_roundtrip(b"exactly16bytes!!", 16);
        stream_roundtrip(b"a little more than one chunk", 16);
        stream_roundtrip(&random_bytes(100 * 1024 + 17, 1), 4096);
    }

    #[test]
    fn header_and_index_roundtrip() {
        let header = HeaderV2 {
            chunk_size: CHUNK_SIZE,
            kdf: Some(KdfParams {
                salt: [5u8; KDF_SALT_LEN],
                m_cost: ARGON2_M_COST_KIB,
                t_cost: ARGON2_T_COST,
                p_cost: ARGON2_P_COST,
            }),
        };
        let bytes = header.to_bytes();
        assert_eq!(bytes.len(), HEADER_LEN_ENCRYPTED as usize);
        assert_eq!(
            HeaderV2 {
                chunk_size: CHUNK_SIZE,
                kdf: None
            }
            .to_bytes()
            .len(),
            HEADER_LEN_PLAIN as usize
        );

        let entries = vec![
            SegmentEntry {
                kind: SEGMENT_KIND_MANIFEST,
                offset: 48,
                plain_len: 10,
                stored_len: 45,
                digest: [1u8; DIGEST_LEN],
            },
            SegmentEntry {
                kind: SEGMENT_KIND_BLOBS,
                offset: 93,
                plain_len: 0,
                stored_len: 35,
                digest: [2u8; DIGEST_LEN],
            },
        ];
        let encoded = encode_index(&entries);
        let decoded = decode_index(&encoded).expect("decode");
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[1].offset, 93);
        assert_eq!(decoded[1].digest, [2u8; DIGEST_LEN]);
        assert!(decode_index(&encoded[..encoded.len() - 1]).is_err());
    }

    #[test]
    fn classify_path_marks_manifest_as_prio() {
        assert_eq!(classify_path("manifest.app"), PathKind::Prio);
        assert_eq!(classify_path("boards/x.board"), PathKind::Prio);
        assert_eq!(classify_path("upload/x.pdf"), PathKind::Secondary);
    }

    #[test]
    fn rebuild_path_keeps_encoded_names_and_rejects_traversal() {
        let base = app_base("app");
        for name in ["Bericht [2024].pdf", "Übersicht (2)#1.pdf"] {
            let encoded = base.clone().join("upload").join(name);
            let listed = encoded
                .to_string()
                .strip_prefix("apps/app/")
                .unwrap()
                .to_string();
            let from_listed = rebuild_path(&base, &listed).unwrap();
            assert_eq!(from_listed, encoded, "listed {name}");
            assert_eq!(
                rebuild_path(&base, &format!("upload/{name}")).unwrap(),
                encoded,
                "raw {name}"
            );
            assert_eq!(display_file_name(&from_listed).as_deref(), Some(name));
        }
        // Traversal is neutralized by re-encoding rather than rejected, so it
        // stays inside the app namespace.
        for rel in ["../escape", "%2E%2E/escape", "a/../../escape"] {
            let resolved = rebuild_path(&base, rel).unwrap();
            assert!(resolved.as_ref().starts_with("apps/app/"), "{rel:?}");
            assert!(!resolved.as_ref().contains("/../"), "{rel:?}");
        }
        // Keys an older release could legitimately have written must restore.
        assert_eq!(
            rebuild_path(&base, "upload/%2E/report.pdf").unwrap(),
            base.clone().join("upload").join(".").join("report.pdf")
        );
        assert_eq!(
            rebuild_path(&base, "upload/%20/report.pdf").unwrap(),
            base.clone().join("upload").join(" ").join("report.pdf")
        );
        assert_eq!(
            rebuild_path(&base, "a//b").unwrap(),
            base.clone().join("a").join("b")
        );
        for rel in ["", "/", "//"] {
            assert!(rebuild_path(&base, rel).is_err(), "{rel:?}");
        }
    }

    #[test]
    fn secret_value_detection() {
        assert!(!secret_has_value(None));
        assert!(!secret_has_value(Some(b"null")));
        assert!(!secret_has_value(Some(b"\"\"")));
        assert!(secret_has_value(Some(b"\"sk-1\"")));
        assert!(secret_has_value(Some(b"42")));
    }

    // ---------- Legacy ZIP writer ----------

    #[test]
    fn zip_write_streaming_writes_files_and_content() {
        let tmp = NamedTempFile::new().expect("tmp");
        let (tx, rx) = std::sync::mpsc::channel::<ZipWriteCmd>();
        let target_file = File::create(tmp.path()).expect("create target file");
        let handle = std::thread::spawn(move || zip_write_streaming(target_file, rx));

        tx.send(ZipWriteCmd::File {
            name: "dir/one.txt".to_string(),
            data: b"first".to_vec(),
        })
        .unwrap();
        tx.send(ZipWriteCmd::File {
            name: "two.bin".to_string(),
            data: vec![1, 2, 3, 4, 5],
        })
        .unwrap();
        tx.send(ZipWriteCmd::Done).unwrap();
        handle.join().expect("join").expect("zip write ok");

        let mut f = File::open(tmp.path()).expect("open zip");
        let mut zip = ZipArchive::new(&mut f).expect("read zip");
        {
            let mut file = zip.by_name("dir/one.txt").expect("missing one.txt");
            let mut buf = String::new();
            file.read_to_string(&mut buf).expect("read one.txt");
            assert_eq!(buf, "first");
        }
        {
            let mut file = zip.by_name("two.bin").expect("missing two.bin");
            let mut buf = Vec::new();
            file.read_to_end(&mut buf).expect("read two.bin");
            assert_eq!(buf, vec![1, 2, 3, 4, 5]);
        }
    }

    #[test]
    fn zip_encrypt_decrypt_roundtrip_integrated() {
        let tmp = NamedTempFile::new().expect("tmp");
        let (tx, rx) = std::sync::mpsc::channel::<ZipWriteCmd>();
        let target_file = File::create(tmp.path()).expect("create target file");
        let writer = std::thread::spawn(move || zip_write_streaming(target_file, rx));
        tx.send(ZipWriteCmd::File {
            name: "dir/one.txt".into(),
            data: b"first".to_vec(),
        })
        .unwrap();
        tx.send(ZipWriteCmd::Done).unwrap();
        writer.join().unwrap().expect("zip write ok");

        let mut zip_bytes = Vec::new();
        File::open(tmp.path())
            .unwrap()
            .read_to_end(&mut zip_bytes)
            .unwrap();
        assert!(is_zip(&zip_bytes));

        let enc = encrypt_bytes("sup3r-secret", &zip_bytes).expect("encrypt");
        assert!(!is_zip(&enc));
        let dec = decrypt_bytes_v1("sup3r-secret", &enc).expect("decrypt");
        assert_eq!(dec, zip_bytes);

        let mut zip = ZipArchive::new(Cursor::new(dec)).expect("valid zip after decrypt");
        let mut file = zip.by_name("dir/one.txt").expect("one.txt present");
        let mut buf = String::new();
        file.read_to_string(&mut buf).expect("read one.txt");
        assert_eq!(buf, "first");
    }

    #[test]
    fn zip_encrypt_decrypt_big_file() {
        let big_data = random_bytes(100 * 1024 * 1024, 42);
        let tmp = NamedTempFile::new().expect("tmp");
        let (tx, rx) = std::sync::mpsc::channel::<ZipWriteCmd>();
        let target_file = File::create(tmp.path()).expect("create target file");
        let writer = std::thread::spawn(move || zip_write_streaming(target_file, rx));
        tx.send(ZipWriteCmd::File {
            name: "big.bin".into(),
            data: big_data.to_vec(),
        })
        .unwrap();
        tx.send(ZipWriteCmd::Done).unwrap();
        writer.join().unwrap().expect("zip write ok");

        let mut zip_bytes = Vec::new();
        File::open(tmp.path())
            .unwrap()
            .read_to_end(&mut zip_bytes)
            .unwrap();
        let enc = encrypt_bytes("bigpass", &zip_bytes).expect("encrypt");
        let dec = decrypt_bytes_v1("bigpass", &enc).expect("decrypt");
        assert_eq!(zip_bytes, dec);

        let mut zip = ZipArchive::new(Cursor::new(dec)).expect("valid zip");
        let mut file = zip.by_name("big.bin").expect("missing big.bin");
        let mut extracted = Vec::new();
        file.read_to_end(&mut extracted).unwrap();
        assert_eq!(extracted, big_data);
    }

    // ---------- v2 round trips ----------

    #[tokio::test]
    async fn export_import_roundtrip_plaintext_archive() {
        assert_roundtrip(None, "app_plain").await;
    }

    #[tokio::test]
    async fn export_import_roundtrip_encrypted_archive() {
        assert_roundtrip(Some("s3cret-pass"), "app_enc").await;
    }

    #[tokio::test]
    async fn export_of_empty_app_still_imports() {
        let (state, _, _) = setup_state();
        let app = make_app("app_empty", state.clone());
        let tempdir = tempfile::tempdir().expect("tmpdir");
        let report = app
            .export_archive_with(
                ExportOptions::default(),
                tempdir.path().join("empty"),
                ArchiveObserver::default(),
            )
            .await
            .expect("export empty app");
        assert_eq!(report.file_count, 0);

        let (import_state, _, _) = setup_state();
        let err = App::import_archive(import_state, report.path, None)
            .await
            .err()
            .expect("no manifest.app means App::load fails, but the archive parses");
        assert!(!err.to_string().contains("panicked"));
    }

    #[tokio::test]
    async fn wrong_password_on_v2_reports_binding_tag_mismatch() {
        let (state, meta_store, storage_store) = setup_state();
        let app_id = "app_wrong_pw";
        let app = make_app(app_id, state.clone());
        app.save().await.expect("seed manifest");
        seed_sample_objects(&meta_store, &storage_store, app_id).await;

        let tempdir = tempfile::tempdir().expect("tmpdir");
        let exported = app
            .export_archive(Some("right".to_string()), tempdir.path().join("enc"))
            .await
            .expect("export");

        let (import_state, _, _) = setup_state();
        let error =
            App::import_archive(import_state.clone(), exported.clone(), Some("wrong".into()))
                .await
                .err()
                .expect("wrong password must fail");
        assert_eq!(error.to_string(), BINDING_TAG_ERROR);

        let error = App::import_archive(import_state, exported, None)
            .await
            .err()
            .expect("missing password must fail");
        assert!(error.to_string().contains("Password required"));
    }

    async fn exported_fixture(
        app_id: &str,
        password: Option<&str>,
    ) -> (tempfile::TempDir, PathBuf) {
        let (state, meta_store, storage_store) = setup_state();
        let app = make_app(app_id, state.clone());
        app.save().await.expect("seed manifest");
        seed_sample_objects(&meta_store, &storage_store, app_id).await;
        seed_encoded_fixture(&storage_store, app_id).await;
        let tempdir = tempfile::tempdir().expect("tmpdir");
        let exported = app
            .export_archive(password.map(str::to_string), tempdir.path().join("fixture"))
            .await
            .expect("export");
        (tempdir, exported)
    }

    #[tokio::test]
    async fn flipped_byte_in_blob_segment_fails_import() {
        for password in [None, Some("pw")] {
            let (_dir, exported) = exported_fixture("app_flip", password).await;
            let layout = read_v2_layout(&mut File::open(&exported).unwrap()).expect("layout");
            let victim = layout.segments[1..]
                .iter()
                .find(|s| s.plain_len > 1024)
                .expect("a populated blob shard");
            let mut bytes = std::fs::read(&exported).unwrap();
            let at = (victim.offset + victim.stored_len / 2) as usize;
            bytes[at] ^= 0x5A;
            std::fs::write(&exported, &bytes).unwrap();

            let (import_state, _, _) = setup_state();
            let error = App::import_archive(import_state, exported, password.map(str::to_string))
                .await
                .err()
                .expect("tampered blob segment must fail");
            assert!(
                error.to_string().contains("corrupted"),
                "unexpected error: {error}"
            );
        }
    }

    #[tokio::test]
    async fn truncated_v2_archive_fails_without_panic() {
        let (_dir, exported) = exported_fixture("app_trunc", Some("pw")).await;
        let bytes = std::fs::read(&exported).unwrap();
        for keep in [0usize, 8, 19, 47, 48, 200, bytes.len() / 2, bytes.len() - 1] {
            std::fs::write(&exported, &bytes[..keep]).unwrap();
            let (import_state, _, _) = setup_state();
            App::import_archive(import_state, exported.clone(), Some("pw".into()))
                .await
                .err()
                .expect("truncated archive must fail");
        }
    }

    // ---------- v1 compatibility ----------

    async fn assert_v1_import(password: Option<&str>, app_id: &str) {
        let (state, meta_store, storage_store) = setup_state();
        let app = make_app(app_id, state.clone());
        app.save().await.expect("seed manifest");
        seed_sample_objects(&meta_store, &storage_store, app_id).await;
        let encoded = ObjectPath::from("apps")
            .join(app_id)
            .join("upload")
            .join("Bericht [2024].pdf");
        storage_store
            .put(
                &encoded,
                PutPayload::from_bytes(Bytes::from_static(b"v1 pdf")),
            )
            .await
            .unwrap();
        let expected_keys = list_keys(&storage_store, app_id).await;

        let tempdir = tempfile::tempdir().expect("tmpdir");
        let exported = legacy_v1::export_archive(
            &app,
            password.map(str::to_string),
            tempdir.path().join("legacy"),
        )
        .await
        .expect("legacy export");
        let mut head = [0u8; 15];
        File::open(&exported)
            .unwrap()
            .read_exact(&mut head)
            .unwrap();
        if password.is_some() {
            assert_eq!(&head, ENC_MAGIC);
        } else {
            assert!(is_zip(&head));
        }

        let (import_state, import_meta, import_storage) = setup_state();
        let imported = App::import_archive(import_state, exported, password.map(str::to_string))
            .await
            .expect("import v1");
        assert_eq!(imported.id, app_id);
        assert_eq!(list_keys(&import_storage, app_id).await, expected_keys);
        assert_eq!(
            get_bytes(&import_storage, &encoded).await,
            Bytes::from_static(b"v1 pdf")
        );
        let config_path = ObjectPath::from("apps").join(app_id).join("config.meta");
        assert_eq!(
            get_bytes(&import_meta, &config_path).await,
            Bytes::from_static(b"meta config")
        );
    }

    #[tokio::test]
    async fn legacy_v1_plain_archive_imports() {
        assert_v1_import(None, "app_v1_plain").await;
    }

    #[tokio::test]
    async fn legacy_v1_encrypted_archive_imports() {
        assert_v1_import(Some("legacy-pw"), "app_v1_enc").await;
        let (state, meta_store, storage_store) = setup_state();
        let app = make_app("app_v1_wrong", state.clone());
        app.save().await.unwrap();
        seed_sample_objects(&meta_store, &storage_store, "app_v1_wrong").await;
        let tempdir = tempfile::tempdir().unwrap();
        let exported = legacy_v1::export_archive(&app, Some("a".into()), tempdir.path().join("x"))
            .await
            .unwrap();
        let (import_state, _, _) = setup_state();
        let error = App::import_archive(import_state, exported, Some("b".into()))
            .await
            .err()
            .expect("wrong v1 password");
        assert_eq!(error.to_string(), BINDING_TAG_ERROR);
    }

    // ---------- Board eviction and preflight ----------

    struct StaleBoardFixture {
        state: Arc<FlowLikeState>,
        _tempdir: tempfile::TempDir,
        exported: PathBuf,
        board_id: String,
        base: ObjectPath,
    }

    /// Exports an app with one board, then registers a modified in-memory
    /// copy of that board so the registry is stale relative to the archive.
    async fn app_with_stale_registered_board(app_id: &str) -> StaleBoardFixture {
        let (state, meta_store, _) = setup_state();
        let board_id = "board_stale".to_string();
        let base = app_base(app_id);
        let meta: Arc<dyn ObjectStore> = meta_store.clone();

        let mut app = make_app(app_id, state.clone());
        let mut board = Board::new(Some(board_id.clone()), base.clone(), state.clone());
        board.name = "Archived".to_string();
        board.mark_changed();
        board.save(Some(meta.clone())).await.expect("save board");
        app.boards.push(board_id.clone());
        app.save().await.expect("save app");

        let tempdir = tempfile::tempdir().expect("tmpdir");
        let exported = app
            .export_archive(None, tempdir.path().join("stale"))
            .await
            .expect("export");

        let mut stale = board.clone();
        stale.name = "Stale".to_string();
        stale.mark_changed();
        stale.save(Some(meta.clone())).await.expect("save stale");
        state
            .register_board(&board_id, Arc::new(Mutex::new(stale.clone())), None)
            .unwrap();
        state
            .register_board(&board_id, Arc::new(Mutex::new(stale)), Some((0, 0, 1)))
            .unwrap();
        assert_eq!(
            Board::load(base.clone(), &board_id, state.clone(), None)
                .await
                .unwrap()
                .name,
            "Stale"
        );
        StaleBoardFixture {
            state,
            _tempdir: tempdir,
            exported,
            board_id,
            base,
        }
    }

    fn assert_board_unregistered(state: &FlowLikeState, board_id: &str) {
        assert!(!state.board_registry().contains_key(board_id));
        assert!(
            !state
                .board_registry()
                .contains_key(&format!("{board_id}-0-0-1"))
        );
    }

    #[tokio::test]
    async fn import_evicts_stale_registered_boards() {
        let fixture = app_with_stale_registered_board("app_stale").await;
        let StaleBoardFixture {
            state,
            exported,
            board_id,
            base,
            ..
        } = &fixture;

        let imported = App::import_archive(state.clone(), exported.clone(), None)
            .await
            .expect("import over open app");
        assert_eq!(imported.boards, vec![board_id.clone()]);
        assert_board_unregistered(state, board_id);
        let restored = Board::load(base.clone(), board_id, state.clone(), None)
            .await
            .expect("load restored board");
        assert_eq!(restored.name, "Archived");
    }

    #[tokio::test]
    async fn cancelled_import_evicts_stale_registered_boards() {
        let fixture = app_with_stale_registered_board("app_stale_cancel").await;
        let (observer, _log) = cancel_on_phase_observer(ArchivePhase::Restoring);
        let error = App::import_archive_with(
            fixture.state.clone(),
            fixture.exported.clone(),
            ImportOptions::default(),
            observer,
        )
        .await
        .err()
        .expect("cancelled import must fail");
        assert_eq!(error.to_string(), ARCHIVE_CANCELLED);
        assert_board_unregistered(&fixture.state, &fixture.board_id);
    }

    #[tokio::test]
    async fn export_preflight_reports_secret_variables_with_values() {
        let (state, meta_store, storage_store) = setup_state();
        let app_id = "app_preflight";
        let base = app_base(app_id);
        let meta: Arc<dyn ObjectStore> = meta_store.clone();

        let mut app = make_app(app_id, state.clone());
        let mut board = Board::new(Some("board_pf".into()), base.clone(), state.clone());
        board.name = "Secrets".to_string();

        let mut with_value = Variable::new("api_key", VariableType::String, ValueType::Normal);
        with_value
            .set_secret(true)
            .set_default_value(flow_like_types::json::json!("sk-live-123"));
        let mut empty = Variable::new("empty_secret", VariableType::String, ValueType::Normal);
        empty
            .set_secret(true)
            .set_default_value(flow_like_types::json::json!(""));
        let mut unset = Variable::new("unset_secret", VariableType::String, ValueType::Normal);
        unset.set_secret(true);
        let mut public = Variable::new("public", VariableType::String, ValueType::Normal);
        public.set_default_value(flow_like_types::json::json!("visible"));
        let with_value_id = with_value.id.clone();
        for variable in [with_value, empty, unset, public] {
            board.variables.insert(variable.id.clone(), variable);
        }
        board.mark_changed();
        board.save(Some(meta)).await.expect("save board");
        app.boards.push(board.id.clone());
        app.save().await.expect("save app");
        seed_sample_objects(&meta_store, &storage_store, app_id).await;

        let versions = base.clone().join("storage").join("db").join("t.lance");
        for (rel, len) in [
            ("_versions/1.manifest", 100usize),
            ("_versions/2.manifest", 120),
            ("_transactions/a.txn", 10),
            ("_transactions/b.txn", 30),
            ("data/x.lance", 500),
        ] {
            let mut path = versions.clone();
            for part in rel.split('/') {
                path = path.join(part);
            }
            storage_store
                .put(&path, PutPayload::from_bytes(Bytes::from(vec![0u8; len])))
                .await
                .unwrap();
        }

        let preflight = app.export_preflight().await.expect("preflight");
        assert_eq!(preflight.secret_variables.len(), 1);
        let secret = &preflight.secret_variables[0];
        assert_eq!(secret.variable_id, with_value_id);
        assert_eq!(secret.variable_name, "api_key");
        assert_eq!(secret.board_id, "board_pf");
        assert_eq!(secret.board_name, "Secrets");

        assert_eq!(preflight.tables.len(), 1);
        let table = &preflight.tables[0];
        assert_eq!(table.name, "t");
        assert_eq!(table.versions, 2);
        assert_eq!(table.total_bytes, 760);
        assert_eq!(table.history_bytes, 100 + 10);
        assert_eq!(preflight.reclaimable_bytes, 110);
        assert!(!preflight.compaction_available);
        assert!(preflight.file_count >= 8);
        assert!(preflight.total_bytes >= 760);
    }

    // ---------- Progress, cancellation, import modes, inspect, visibility ----------

    type ProgressLog = Arc<std::sync::Mutex<Vec<ArchiveProgress>>>;

    fn collecting_observer() -> (ArchiveObserver, ProgressLog) {
        let log: ProgressLog = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = log.clone();
        let observer = ArchiveObserver {
            progress: Some(Arc::new(move |progress| {
                sink.lock().unwrap().push(progress);
            })),
            cancel: None,
        };
        (observer, log)
    }

    fn cancelled_observer() -> ArchiveObserver {
        let token = CancellationToken::new();
        token.cancel();
        ArchiveObserver {
            progress: None,
            cancel: Some(token),
        }
    }

    /// Cancels the token from inside the sink the first time `phase` is
    /// reported; the reporter emits synchronously, so the next `check()`
    /// after that phase change is guaranteed to observe the cancel.
    fn cancel_on_phase_observer(phase: ArchivePhase) -> (ArchiveObserver, ProgressLog) {
        let (mut observer, log) = collecting_observer();
        let token = CancellationToken::new();
        let cancel = token.clone();
        let record = observer.progress.take().expect("collecting sink");
        observer.progress = Some(Arc::new(move |progress: ArchiveProgress| {
            if progress.phase == phase {
                cancel.cancel();
            }
            record(progress);
        }));
        observer.cancel = Some(token);
        (observer, log)
    }

    fn assert_no_done(log: &ProgressLog) {
        assert!(
            log.lock()
                .unwrap()
                .iter()
                .all(|p| p.phase != ArchivePhase::Done)
        );
    }

    async fn app_object_count(store: &Arc<InMemory>, app_id: &str) -> usize {
        let store: Arc<dyn ObjectStore> = store.clone();
        list_store_files(&store, &app_base(app_id))
            .await
            .expect("list")
            .len()
    }

    fn assert_progress_shape(log: &[ArchiveProgress], expected: &[ArchivePhase]) {
        assert_eq!(log.last().map(|p| p.phase), Some(ArchivePhase::Done));
        let mut previous: Option<&ArchiveProgress> = None;
        for progress in log {
            if let Some(prev) = previous
                && prev.phase == progress.phase
            {
                assert!(
                    progress.done_bytes >= prev.done_bytes,
                    "{:?} bytes regressed",
                    progress.phase
                );
                assert!(
                    progress.done_files >= prev.done_files,
                    "{:?} files regressed",
                    progress.phase
                );
            }
            previous = Some(progress);
        }
        for phase in expected {
            let last = log
                .iter()
                .filter(|p| p.phase == *phase)
                .next_back()
                .unwrap_or_else(|| panic!("missing phase {phase:?}"));
            assert_eq!(
                last.done_bytes, last.total_bytes,
                "{phase:?} bytes incomplete"
            );
            assert_eq!(
                last.done_files, last.total_files,
                "{phase:?} files incomplete"
            );
        }
    }

    async fn seeded_app(app_id: &str) -> (Arc<FlowLikeState>, Arc<InMemory>, Arc<InMemory>, App) {
        let (state, meta_store, storage_store) = setup_state();
        let app = make_app(app_id, state.clone());
        app.save().await.expect("seed manifest");
        seed_sample_objects(&meta_store, &storage_store, app_id).await;
        (state, meta_store, storage_store, app)
    }

    async fn total_size(store: &Arc<InMemory>, app_id: &str) -> u64 {
        let store: Arc<dyn ObjectStore> = store.clone();
        list_store_files(&store, &app_base(app_id))
            .await
            .expect("list")
            .iter()
            .map(|(_, size)| *size)
            .sum()
    }

    #[tokio::test]
    async fn progress_sink_sees_every_phase_and_ends_with_done() {
        use ArchivePhase::*;
        let app_id = "app_progress";
        let (_state, _meta, storage_store, app) = seeded_app(app_id).await;
        let fixture = seed_encoded_fixture(&storage_store, app_id).await;
        let tempdir = tempfile::tempdir().expect("tmpdir");

        let (observer, log) = collecting_observer();
        let report = app
            .export_archive_with(
                ExportOptions::default(),
                tempdir.path().join("progress"),
                observer,
            )
            .await
            .expect("export");
        let export_log = log.lock().unwrap().clone();
        assert_progress_shape(&export_log, &[Listing, Packing, Writing, Finalizing]);
        let packing = export_log
            .iter()
            .filter(|p| p.phase == Packing)
            .next_back()
            .unwrap();
        assert_eq!(packing.total_files, fixture.len() as u64 + 3);
        assert!(packing.total_bytes > 6 * 1024 * 1024);
        let writing = export_log
            .iter()
            .filter(|p| p.phase == Writing)
            .next_back()
            .unwrap();
        assert!(writing.total_bytes > 0 && writing.total_bytes < report.bytes_written);

        let (observer, log) = collecting_observer();
        let (import_state, _, _) = setup_state();
        let imported = App::import_archive_with(
            import_state,
            report.path,
            ImportOptions::default(),
            observer,
        )
        .await
        .expect("import");
        assert_eq!(imported.mode, ImportMode::Merge);
        assert_eq!(imported.restored_files, report.file_count);
        assert_eq!(imported.skipped_files, 0);
        assert_eq!(imported.deleted_files, 0);
        let import_log = log.lock().unwrap().clone();
        assert_progress_shape(&import_log, &[Reading, Planning, Restoring, Finalizing]);
        assert!(!import_log.iter().any(|p| p.phase == Cleaning));
        let restoring = import_log
            .iter()
            .filter(|p| p.phase == Restoring)
            .next_back()
            .unwrap();
        assert_eq!(restoring.total_files, report.file_count);
        assert_eq!(restoring.total_bytes, packing.total_bytes);
    }

    #[tokio::test]
    async fn pre_cancelled_token_aborts_export_and_import() {
        let app_id = "app_cancel";
        let (state, _, _, app) = seeded_app(app_id).await;
        let tempdir = tempfile::tempdir().expect("tmpdir");
        let target = tempdir.path().join("cancelled");

        let error = app
            .export_archive_with(
                ExportOptions::default(),
                target.clone(),
                cancelled_observer(),
            )
            .await
            .err()
            .expect("cancelled export must fail");
        assert_eq!(error.to_string(), ARCHIVE_CANCELLED);
        assert!(!target.with_extension("flow-app").exists());
        assert!(std::fs::read_dir(tempdir.path()).unwrap().next().is_none());

        let exported = app
            .export_archive(None, tempdir.path().join("ok"))
            .await
            .expect("export");
        let error = App::import_archive_with(
            state,
            exported,
            ImportOptions::default(),
            cancelled_observer(),
        )
        .await
        .err()
        .expect("cancelled import must fail");
        assert_eq!(error.to_string(), ARCHIVE_CANCELLED);
    }

    #[tokio::test]
    async fn cancelled_export_keeps_a_pre_existing_target() {
        let (_state, _, _, app) = seeded_app("app_cancel_existing").await;
        let tempdir = tempfile::tempdir().expect("tmpdir");
        let target = tempdir.path().join("existing.flow-app");
        std::fs::write(&target, b"previous archive").unwrap();

        let error = app
            .export_archive_with(
                ExportOptions::default(),
                target.clone(),
                cancelled_observer(),
            )
            .await
            .err()
            .expect("cancelled export must fail");
        assert_eq!(error.to_string(), ARCHIVE_CANCELLED);
        assert_eq!(std::fs::read(&target).unwrap(), b"previous archive");
    }

    #[tokio::test]
    async fn export_cancelled_while_finalizing_removes_created_target() {
        let (_state, _, _, app) = seeded_app("app_cancel_finalizing").await;
        let tempdir = tempfile::tempdir().expect("tmpdir");
        let target = tempdir.path().join("late");
        let (observer, log) = cancel_on_phase_observer(ArchivePhase::Finalizing);

        let error = app
            .export_archive_with(ExportOptions::default(), target.clone(), observer)
            .await
            .err()
            .expect("cancelled export must fail");
        assert_eq!(error.to_string(), ARCHIVE_CANCELLED);
        assert!(
            log.lock()
                .unwrap()
                .iter()
                .any(|p| p.phase == ArchivePhase::Writing)
        );
        assert_no_done(&log);
        assert!(!target.with_extension("flow-app").exists());
        assert!(std::fs::read_dir(tempdir.path()).unwrap().next().is_none());
    }

    #[tokio::test]
    async fn failed_assembly_removes_the_file_it_created() {
        let tempdir = tempfile::tempdir().expect("tmpdir");
        let target = tempdir.path().join("broken.flow-app");
        let mut short = tempfile().unwrap();
        short.write_all(b"short").unwrap();
        let sources = vec![SegmentSource {
            kind: SEGMENT_KIND_BLOBS,
            file: short,
            plain_len: 4096,
        }];
        let header = HeaderV2 {
            chunk_size: CHUNK_SIZE,
            kdf: None,
        };
        let reporter = ProgressReporter::new(ArchiveObserver::default());

        let error = assemble_archive(target.clone(), header, sources, None, 1, reporter)
            .await
            .err()
            .expect("short source must fail");
        assert!(!is_cancelled_error(&error), "{error}");
        assert!(
            error.to_string().contains("Segment 0 copy failed"),
            "{error}"
        );
        assert!(!target.exists());
        assert!(
            std::fs::read_dir(tempdir.path()).unwrap().next().is_none(),
            "the staging file must not survive a failed assembly"
        );
    }

    #[tokio::test]
    async fn spooled_blobs_hash_and_pack_like_inline_ones() {
        let store: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
        let path = Path::from("spooled.bin");
        let payload: Vec<u8> = (0..(2 * COPY_BUFFER_LEN + 17))
            .map(|i| (i % 251) as u8)
            .collect();
        store
            .put(&path, PutPayload::from_bytes(Bytes::from(payload.clone())))
            .await
            .expect("seed blob");

        let listed = ListedFile {
            store: StoreKind::Storage,
            path,
            rel: "spooled.bin".into(),
            size: payload.len() as u64,
        };
        let budget = Arc::new(Semaphore::new(BUDGET_PERMITS as usize));
        let reporter = ProgressReporter::new(ArchiveObserver::default());
        let blob = spool_and_hash(store, listed, budget, reporter.clone())
            .await
            .expect("spool");
        assert_eq!(blob.len, payload.len() as u64);
        assert_eq!(blob.hash, blake3_hex(&payload));
        assert!(matches!(blob.body, BlobBody::Spooled(_)));

        let (tx, rx) = mpsc::channel::<ShardEntry>(1);
        let writer = task::spawn_blocking({
            let reporter = reporter.clone();
            move || write_shard_zip(rx, reporter)
        });
        tx.send(ShardEntry {
            name: blob.hash.clone(),
            body: blob.body,
            _permit: blob.permit,
        })
        .await
        .expect("send shard entry");
        drop(tx);
        let shard = writer.await.expect("writer joins").expect("shard written");

        let mut archive = ZipArchive::new(shard).expect("zip");
        let mut packed = Vec::new();
        archive
            .by_name(&blob.hash)
            .expect("packed blob")
            .read_to_end(&mut packed)
            .expect("read packed blob");
        assert_eq!(packed, payload);
    }

    #[test]
    fn archive_suffixes_are_idempotent_and_keep_dotted_names() {
        let cases = [
            ("MyApp", true, "MyApp.enc.flow-app"),
            ("MyApp", false, "MyApp.flow-app"),
            ("MyApp.flow-app", true, "MyApp.enc.flow-app"),
            ("MyApp.flow-app", false, "MyApp.flow-app"),
            ("MyApp.enc.flow-app", true, "MyApp.enc.flow-app"),
            ("MyApp.enc.flow-app", false, "MyApp.flow-app"),
            ("backup 2026.01.15", true, "backup 2026.01.15.enc.flow-app"),
            ("backup 2026.01.15", false, "backup 2026.01.15.flow-app"),
            (".flow-app", false, ".flow-app"),
            (".flow-app", true, ".enc.flow-app"),
            (".enc.flow-app", true, ".enc.flow-app"),
        ];
        for (name, encrypted, expected) in cases {
            let mut path = PathBuf::from("/tmp").join(name);
            apply_archive_suffix(&mut path, encrypted);
            assert_eq!(path.file_name().unwrap(), expected, "{name} ({encrypted})");
            let once = path.clone();
            apply_archive_suffix(&mut path, encrypted);
            assert_eq!(path, once, "{name} ({encrypted}) is not idempotent");
        }
    }

    #[tokio::test]
    async fn export_never_double_suffixes_an_already_named_target() {
        let (_state, _, _, app) = seeded_app("app_suffix").await;
        let tempdir = tempfile::tempdir().expect("tmpdir");

        let target = tempdir.path().join("MyApp.enc.flow-app");
        let report = app
            .export_archive_with(
                ExportOptions {
                    password: Some("pw".into()),
                    compact_tables: false,
                },
                target.clone(),
                ArchiveObserver::default(),
            )
            .await
            .expect("encrypted export");
        assert_eq!(report.path, target);
        assert!(target.exists());
        assert!(report.bytes_written > 0);

        let plain = app
            .export_archive_with(
                ExportOptions::default(),
                tempdir.path().join("Other.enc.flow-app"),
                ArchiveObserver::default(),
            )
            .await
            .expect("plain export");
        assert_eq!(
            plain.path.file_name().unwrap().to_str().unwrap(),
            "Other.flow-app"
        );

        let dotted = app
            .export_archive_with(
                ExportOptions {
                    password: Some("pw".into()),
                    compact_tables: false,
                },
                tempdir.path().join("backup 2026.01.15"),
                ArchiveObserver::default(),
            )
            .await
            .expect("dotted export");
        assert_eq!(
            dotted.path.file_name().unwrap().to_str().unwrap(),
            "backup 2026.01.15.enc.flow-app"
        );
        assert!(dotted.path.exists());
    }

    #[tokio::test]
    async fn export_cancelled_while_writing_keeps_a_pre_existing_archive() {
        let (_state, _, _, app) = seeded_app("app_cancel_writing").await;
        let tempdir = tempfile::tempdir().expect("tmpdir");
        let target = tempdir.path().join("existing.flow-app");
        std::fs::write(&target, b"previous archive").unwrap();
        let (observer, log) = cancel_on_phase_observer(ArchivePhase::Writing);

        let error = app
            .export_archive_with(ExportOptions::default(), target.clone(), observer)
            .await
            .err()
            .expect("cancelled export must fail");
        assert_eq!(error.to_string(), ARCHIVE_CANCELLED);
        assert_no_done(&log);
        assert_eq!(std::fs::read(&target).unwrap(), b"previous archive");
        let leftovers: Vec<_> = std::fs::read_dir(tempdir.path())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .filter(|name| name != "existing.flow-app")
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    #[test]
    fn segment_probes_abort_between_chunks() {
        use std::cell::Cell;
        let plain = vec![7u8; 64];
        let chunk = 16u32;
        let key = [1u8; 32];
        let never = || false;
        let second_call = || {
            let calls = Cell::new(0u32);
            move || {
                calls.set(calls.get() + 1);
                calls.get() == 2
            }
        };

        let mut encrypted = Vec::new();
        encrypt_segment(
            &mut Cursor::new(plain.clone()),
            64,
            &key,
            3,
            chunk,
            &never,
            &mut encrypted,
        )
        .unwrap();

        let mut copied = Vec::new();
        let error = copy_segment(
            &mut Cursor::new(plain.clone()),
            64,
            chunk,
            &second_call(),
            &mut copied,
        )
        .err()
        .expect("copy aborts");
        assert_eq!(error.to_string(), ARCHIVE_CANCELLED);
        assert_eq!(copied.len(), chunk as usize);
        assert!(is_cancelled_error(&anyhow!(
            "Segment 0 copy failed: {error}"
        )));

        let error = encrypt_segment(
            &mut Cursor::new(plain),
            64,
            &key,
            3,
            chunk,
            &second_call(),
            &mut Vec::new(),
        )
        .err()
        .expect("encrypt aborts");
        assert_eq!(error.to_string(), ARCHIVE_CANCELLED);

        let error = decrypt_segment(
            &mut Cursor::new(encrypted.clone()),
            encrypted.len() as u64,
            &key,
            3,
            chunk,
            &second_call(),
            &mut Vec::new(),
        )
        .err()
        .expect("decrypt aborts");
        assert_eq!(error.to_string(), ARCHIVE_CANCELLED);
        assert!(is_cancelled_error(&anyhow!(
            "Shard writer 1 failed: {error}"
        )));
    }

    #[tokio::test]
    async fn import_cancelled_before_restore_writes_nothing() {
        let app_id = "app_cancel_restore";
        let (_dir, exported) = exported_fixture(app_id, None).await;
        let (state, meta_store, storage_store) = setup_state();
        let (observer, log) = cancel_on_phase_observer(ArchivePhase::Restoring);

        let error =
            App::import_archive_with(state.clone(), exported, ImportOptions::default(), observer)
                .await
                .err()
                .expect("cancelled import must fail");
        assert_eq!(error.to_string(), ARCHIVE_CANCELLED);
        assert_no_done(&log);
        assert_eq!(app_object_count(&meta_store, app_id).await, 0);
        assert_eq!(app_object_count(&storage_store, app_id).await, 0);
        assert!(App::load(app_id.to_string(), state).await.is_err());
    }

    #[tokio::test]
    async fn import_cancelled_while_finalizing_keeps_restored_files() {
        let app_id = "app_cancel_finalize";
        let (_dir, exported) = exported_fixture(app_id, None).await;
        let (state, meta_store, storage_store) = setup_state();
        let (observer, log) = cancel_on_phase_observer(ArchivePhase::Finalizing);

        let error =
            App::import_archive_with(state.clone(), exported, ImportOptions::default(), observer)
                .await
                .err()
                .expect("cancelled import must fail");
        assert_eq!(error.to_string(), ARCHIVE_CANCELLED);
        assert_no_done(&log);
        let fully_restored = log
            .lock()
            .unwrap()
            .iter()
            .any(|p| p.phase == ArchivePhase::Restoring && p.done_files == p.total_files);
        assert!(fully_restored);
        assert!(app_object_count(&meta_store, app_id).await > 0);
        assert!(app_object_count(&storage_store, app_id).await > 0);
        let restored = App::load(app_id.to_string(), state)
            .await
            .expect("restored manifest is readable");
        assert_eq!(restored.id, app_id);
    }

    #[tokio::test]
    async fn replace_mode_deletes_local_extras_and_merge_keeps_them() {
        let app_id = "app_modes";
        let (_dir, exported) = exported_fixture(app_id, None).await;
        let fixture = encoded_fixture(app_id);
        let manifest_entries = fixture.len() as u64 + 3;

        let (state, _meta, storage) = setup_state();
        let extra = app_base(app_id).join("extra.txt");
        let inert_traversal = join_object_path(&app_base(app_id), "../Übersicht (2)#1.pdf");
        assert_eq!(
            inert_traversal.as_ref(),
            "apps/app_modes/%2E%2E/%C3%9Cbersicht (2)%231.pdf"
        );
        let extras = [&extra, &inert_traversal];
        for path in extras {
            storage
                .put(
                    path,
                    PutPayload::from_bytes(Bytes::from_static(b"local only")),
                )
                .await
                .unwrap();
        }

        let merged = App::import_archive_with(
            state.clone(),
            exported.clone(),
            ImportOptions::default(),
            ArchiveObserver::default(),
        )
        .await
        .expect("merge import");
        assert_eq!(merged.mode, ImportMode::Merge);
        assert_eq!(merged.deleted_files, 0);
        assert_eq!(merged.restored_files, manifest_entries);
        for path in extras {
            assert!(
                storage.head(path).await.is_ok(),
                "merge keeps local extra {path}"
            );
        }

        let (observer, log) = collecting_observer();
        let replaced = App::import_archive_with(
            state,
            exported,
            ImportOptions {
                password: None,
                mode: ImportMode::Replace,
            },
            observer,
        )
        .await
        .expect("replace import");
        assert_eq!(replaced.mode, ImportMode::Replace);
        assert_eq!(replaced.deleted_files, 2);
        assert_eq!(
            replaced.restored_files + replaced.skipped_files,
            manifest_entries
        );
        assert!(replaced.skipped_files >= fixture.len() as u64 + 2);
        for path in extras {
            assert!(
                storage.head(path).await.is_err(),
                "replace removes extra {path}"
            );
        }
        for (path, bytes) in &fixture {
            assert_eq!(&get_bytes(&storage, path).await, bytes, "{path}");
        }
        let cleaning = log
            .lock()
            .unwrap()
            .iter()
            .filter(|p| p.phase == ArchivePhase::Cleaning)
            .next_back()
            .cloned()
            .expect("cleaning phase reported");
        assert_eq!((cleaning.done_files, cleaning.total_files), (2, 2));
    }

    #[tokio::test]
    async fn inspect_reports_layout_for_every_variant() {
        let app_id = "app_inspect";
        let (state, meta_store, storage_store, app) = seeded_app(app_id).await;
        let expected_bytes =
            total_size(&meta_store, app_id).await + total_size(&storage_store, app_id).await;
        let tempdir = tempfile::tempdir().expect("tmpdir");
        let plain = app
            .export_archive(None, tempdir.path().join("plain"))
            .await
            .unwrap();
        let encrypted = app
            .export_archive(Some("pw".into()), tempdir.path().join("enc"))
            .await
            .unwrap();
        let v1_plain = legacy_v1::export_archive(&app, None, tempdir.path().join("v1"))
            .await
            .unwrap();
        let v1_encrypted =
            legacy_v1::export_archive(&app, Some("pw".into()), tempdir.path().join("v1enc"))
                .await
                .unwrap();
        let (fresh, _, _) = setup_state();

        let info = App::inspect_archive(state.clone(), plain.clone(), None)
            .await
            .expect("inspect v2 plain");
        assert_eq!(info.format_version, 2);
        assert!(!info.encrypted);
        assert_eq!(info.app_id.as_deref(), Some(app_id));
        assert!(info.created_at.is_some());
        assert_eq!(info.file_count, Some(3));
        assert_eq!(info.total_bytes, Some(expected_bytes));
        assert!(info.exists_locally);
        assert!(matches!(
            info.local_visibility,
            Some(AppVisibility::Private)
        ));

        let info = App::inspect_archive(fresh.clone(), plain, None)
            .await
            .expect("inspect without local app");
        assert!(!info.exists_locally);
        assert!(info.local_visibility.is_none());

        let sealed = App::inspect_archive(fresh.clone(), encrypted.clone(), None)
            .await
            .expect("sealed inspect must not error");
        assert_eq!(sealed.format_version, 2);
        assert!(sealed.encrypted);
        assert!(sealed.app_id.is_none());
        assert!(sealed.created_at.is_none());
        assert!(sealed.file_count.is_none());
        assert!(sealed.total_bytes.is_none());
        assert!(!sealed.exists_locally);

        let opened = App::inspect_archive(state.clone(), encrypted.clone(), Some("pw".into()))
            .await
            .expect("inspect with password");
        assert!(opened.encrypted);
        assert_eq!(opened.app_id.as_deref(), Some(app_id));
        assert_eq!(opened.file_count, Some(3));
        assert_eq!(opened.total_bytes, Some(expected_bytes));
        assert!(opened.exists_locally);

        let wrong = App::inspect_archive(state.clone(), encrypted, Some("nope".into()))
            .await
            .err()
            .expect("wrong password");
        assert_eq!(wrong.to_string(), BINDING_TAG_ERROR);

        let v1 = App::inspect_archive(state.clone(), v1_plain, None)
            .await
            .expect("inspect v1 plain");
        assert_eq!(v1.format_version, 1);
        assert!(!v1.encrypted);
        assert_eq!(v1.app_id.as_deref(), Some(app_id));
        assert_eq!(v1.file_count, Some(3));
        assert_eq!(v1.total_bytes, Some(expected_bytes));
        assert!(v1.exists_locally);

        let v1_sealed = App::inspect_archive(state.clone(), v1_encrypted.clone(), None)
            .await
            .expect("inspect v1 sealed");
        assert_eq!(v1_sealed.format_version, 1);
        assert!(v1_sealed.encrypted);
        assert!(v1_sealed.app_id.is_none());

        let v1_opened = App::inspect_archive(state, v1_encrypted, Some("pw".into()))
            .await
            .expect("inspect v1 with password");
        assert_eq!(v1_opened.app_id.as_deref(), Some(app_id));
        assert_eq!(v1_opened.file_count, Some(3));
    }

    #[tokio::test]
    async fn import_visibility_is_offline_for_fresh_and_kept_for_existing() {
        let app_id = "app_visibility";
        let (_dir, exported) = exported_fixture(app_id, None).await;

        let (fresh, _, _) = setup_state();
        let imported = App::import_archive(fresh.clone(), exported.clone(), None)
            .await
            .expect("fresh import");
        assert!(matches!(imported.visibility, AppVisibility::Offline));
        let reloaded = App::load(app_id.to_string(), fresh).await.unwrap();
        assert!(matches!(reloaded.visibility, AppVisibility::Offline));

        let (state, _, _) = setup_state();
        let mut existing = make_app(app_id, state.clone());
        existing.visibility = AppVisibility::Public;
        existing.save().await.unwrap();
        let imported = App::import_archive(state.clone(), exported, None)
            .await
            .expect("import over existing app");
        assert!(matches!(imported.visibility, AppVisibility::Public));
        let reloaded = App::load(app_id.to_string(), state).await.unwrap();
        assert!(matches!(reloaded.visibility, AppVisibility::Public));
    }
}
