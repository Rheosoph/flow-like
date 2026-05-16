/// TODO: optimize this. Implement stream encryption / decryption and Slice Readers for more memory efficiency.
/// Also make sure that we use `async` I/O everywhere.
use super::App;
use crate::state::FlowLikeState;
use argon2::{Algorithm, Argon2, Params, Version};
use flow_like_storage::{
    Path, blake3,
    object_store::{ObjectStore, PutPayload},
};
use flow_like_types::anyhow;
use flow_like_types::{Bytes, bail, rand::TryRngCore, tokio::task};
use futures::{StreamExt, TryStreamExt};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    path::PathBuf,
    sync::Arc,
    time::SystemTime,
};
use tempfile::tempfile;
use zeroize::Zeroize;
use zip::{ZipArchive, ZipWriter};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum PathKind {
    Prio,
    Secondary,
}

// Sorts the files into separate containers based on their path.
fn classify_path(path: &str) -> PathKind {
    if path.ends_with("/app.manifest")
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
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExportManifest {
    pub version: u32,
    pub app_id: String,
    pub created_at: u64,
    pub prio: HashMap<String, ManifestEntry>,
    pub secondary: HashMap<String, ManifestEntry>,
}

#[derive(Clone, Copy)]
struct TrailerV1 {
    manifest_size: u64,
    prio_size: u64,
    secondary_size: u64,
}

impl TrailerV1 {
    fn new() -> Self {
        Self {
            manifest_size: 0,
            prio_size: 0,
            secondary_size: 0,
        }
    }

    fn to_bytes(&self) -> [u8; 24] {
        let mut buf = [0u8; 24];
        buf[0..8].copy_from_slice(&self.manifest_size.to_le_bytes());
        buf[8..16].copy_from_slice(&self.prio_size.to_le_bytes());
        buf[16..24].copy_from_slice(&self.secondary_size.to_le_bytes());
        buf
    }

    fn from_bytes(bytes: &[u8; 24]) -> Self {
        Self {
            manifest_size: u64::from_le_bytes(bytes[0..8].try_into().unwrap()),
            prio_size: u64::from_le_bytes(bytes[8..16].try_into().unwrap()),
            secondary_size: u64::from_le_bytes(bytes[16..24].try_into().unwrap()),
        }
    }
}

const ENC_MAGIC: &[u8] = b"FLOWAPP_CHACHA2";
const SALT_LEN: usize = 16;
const XNONCE_LEN: usize = 24;

const ARGON2_M_COST_KIB: u32 = 64 * 1024;
const ARGON2_T_COST: u32 = 3;
const ARGON2_P_COST: u32 = 1;

// ===== Binding (archive-level) MAC =====
const BINDING_SALT_LEN: usize = 16;
const BINDING_TAG_LEN: usize = 32;
const BINDING_CONTEXT: &[u8] = b"flow-archive|v1";

fn derive_binding_key(
    password: &str,
    salt: &[u8; BINDING_SALT_LEN],
) -> flow_like_types::Result<[u8; 32]> {
    let params = Params::new(ARGON2_M_COST_KIB, ARGON2_T_COST, ARGON2_P_COST, None)
        .map_err(|e| anyhow!(e.to_string()))?;
    let kdf = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = [0u8; 32];
    kdf.hash_password_into(password.as_bytes(), salt, &mut key)
        .map_err(|e| anyhow!(e.to_string()))?;
    Ok(key)
}

fn compute_binding_tag(
    key: &[u8; 32],
    trailer_bytes: &[u8; 24],
    manifest_ct: &[u8],
    prio_ct: &[u8],
    secondary_ct: &[u8],
) -> [u8; BINDING_TAG_LEN] {
    let mut hasher = blake3::Hasher::new_keyed(key);
    hasher.update(BINDING_CONTEXT);
    hasher.update(trailer_bytes);
    hasher.update(manifest_ct);
    hasher.update(prio_ct);
    hasher.update(secondary_ct);
    let h = hasher.finalize();
    let mut out = [0u8; BINDING_TAG_LEN];
    out.copy_from_slice(h.as_bytes());
    out
}

async fn obj_exists_with_size(
    store: &Arc<dyn ObjectStore>,
    path: &Path,
    expect_size: u64,
) -> flow_like_types::Result<Option<bool>> {
    match store.head(path).await {
        Ok(obj) => {
            let size = obj.size;
            Ok(Some(size == expect_size))
        }
        Err(_) => Ok(None),
    }
}

fn blake3_hash(data: &[u8]) -> String {
    blake3::hash(data).to_hex().to_string().to_lowercase()
}

async fn stream_blake3_of_object(
    store: &Arc<dyn ObjectStore>,
    path: &Path,
) -> flow_like_types::Result<String> {
    let obj = store.get(path).await?;
    let mut hasher = blake3::Hasher::new();

    let stream = obj.into_stream();
    let mut stream = stream.map_err(|e| anyhow!("Failed to read object stream: {}", e));

    while let Some(Ok(chunk)) = stream.next().await {
        hasher.update(&chunk);
    }
    Ok(hasher.finalize().to_hex().to_string().to_lowercase())
}

enum ZipWriteCmd {
    File { name: String, data: Vec<u8> },
    Done,
}

fn zip_write_streaming(
    mut file: File,
    rx: std::sync::mpsc::Receiver<ZipWriteCmd>,
) -> flow_like_types::Result<()> {
    let mut zw = ZipWriter::new(&mut file);
    let opts = zip::write::FileOptions::<zip::write::ExtendedFileOptions>::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .large_file(true);

    while let Ok(cmd) = rx.recv() {
        match cmd {
            ZipWriteCmd::File { name, data } => {
                zw.start_file(name, opts.clone())?;
                zw.write_all(&data)?;
            }
            ZipWriteCmd::Done => {
                break;
            }
        }
    }

    zw.finish()?;
    file.flush()?;
    Ok(())
}

async fn collect_store_files(
    store: Arc<dyn ObjectStore>,
    root: &Path,
) -> flow_like_types::Result<Vec<(Path, u64)>> {
    let mut out = Vec::new();
    let mut stream = store.list(Some(root)).boxed();
    while let Some(item) = stream.try_next().await? {
        out.push((item.location, item.size));
    }
    Ok(out)
}

async fn read_store_file_bytes(
    store: Arc<dyn ObjectStore>,
    path: &Path,
) -> flow_like_types::Result<Vec<u8>> {
    let obj = store.get(path).await?;
    let b = obj.bytes().await?;
    Ok(b.to_vec())
}

fn encrypt_bytes(password: &str, plain: &[u8]) -> flow_like_types::Result<Vec<u8>> {
    use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce, aead::Aead};
    use flow_like_types::rand::rngs::OsRng;

    let mut salt = [0u8; SALT_LEN];
    let mut nonce = [0u8; XNONCE_LEN];
    let mut rng = OsRng;
    rng.try_fill_bytes(&mut salt)?;
    rng.try_fill_bytes(&mut nonce)?;

    let params = Params::new(ARGON2_M_COST_KIB, ARGON2_T_COST, ARGON2_P_COST, None)
        .map_err(|e| anyhow!(e.to_string()))?;
    let kdf = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);

    let mut key = [0u8; 32];
    kdf.hash_password_into(password.as_bytes(), &salt, &mut key)
        .map_err(|e| anyhow!(e.to_string()))?;

    let cipher = XChaCha20Poly1305::new_from_slice(&key).map_err(|e| anyhow!(e))?;
    let nonce = XNonce::from(nonce);
    let ciphertext = cipher.encrypt(&nonce, plain).map_err(|e| anyhow!(e))?;

    key.zeroize();
    let mut out = Vec::with_capacity(ENC_MAGIC.len() + SALT_LEN + XNONCE_LEN + ciphertext.len());
    out.extend_from_slice(ENC_MAGIC);
    out.extend_from_slice(&salt);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

fn decrypt_bytes(password: &str, data: &[u8]) -> flow_like_types::Result<Vec<u8>> {
    use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce, aead::Aead};

    if data.len() < ENC_MAGIC.len() + SALT_LEN + XNONCE_LEN {
        return Err(anyhow!("Invalid encrypted archive"));
    }
    let (magic, rest) = data.split_at(ENC_MAGIC.len());
    if magic != ENC_MAGIC {
        return Err(anyhow!("Invalid encrypted archive header"));
    }
    let (salt, rest) = rest.split_at(SALT_LEN);
    let (nonce, ciphertext) = rest.split_at(XNONCE_LEN);

    let params = Params::new(ARGON2_M_COST_KIB, ARGON2_T_COST, ARGON2_P_COST, None)
        .map_err(|e| anyhow!(e.to_string()))?;
    let kdf = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);

    let mut key = [0u8; 32];
    kdf.hash_password_into(password.as_bytes(), salt, &mut key)
        .map_err(|e| anyhow!(e.to_string()))?;

    let cipher = XChaCha20Poly1305::new_from_slice(&key).map_err(|e| anyhow!(e))?;
    key.zeroize();
    let nonce: [u8; XNONCE_LEN] = nonce
        .try_into()
        .map_err(|_| anyhow!("Invalid nonce length"))?;
    let nonce = XNonce::from(nonce);
    let plain = cipher
        .decrypt(&nonce, ciphertext)
        .map_err(|_| anyhow!("Decryption failed"))?;
    Ok(plain)
}

fn now_unix() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

impl App {
    pub async fn export_archive(
        &self,
        password: Option<String>,
        mut target_file: PathBuf,
    ) -> flow_like_types::Result<PathBuf> {
        target_file.set_extension(if password.is_some() {
            "enc.flow-app"
        } else {
            "flow-app"
        });

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

        let base = Path::from("apps").child(self.id.clone());
        let meta_files = collect_store_files(meta.clone(), &base).await?;
        let storage_files = collect_store_files(storage.clone(), &base).await?;

        if let Some(parent) = target_file.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut manifest = ExportManifest {
            version: 1,
            app_id: self.id.clone(),
            created_at: now_unix(),
            prio: HashMap::new(),
            secondary: HashMap::new(),
        };

        let base_str = base.to_string();
        let base_prefix = if base_str.ends_with('/') {
            base_str
        } else {
            format!("{}/", base_str)
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

        let prio_handle = task::spawn_blocking({
            let prio_zip = prio_zip;
            move || zip_write_streaming(prio_zip, prio_rx)
        });

        let secondary_handle = task::spawn_blocking({
            let secondary_zip = secondary_zip;
            move || zip_write_streaming(secondary_zip, secondary_rx)
        });

        let manifest_handle = task::spawn_blocking({
            let manifest_file = manifest_file;
            move || zip_write_streaming(manifest_file, manifest_rx)
        });

        let mut prio_deduplicator = HashSet::new();
        let mut secondary_deduplicator = HashSet::new();

        for (path, size) in meta_files {
            let full = path.to_string();
            let rel = full.strip_prefix(&base_prefix).unwrap_or(&full).to_string();
            let bytes = read_store_file_bytes(meta.clone(), &path).await?;
            let hash = blake3_hash(&bytes);
            let classification = classify_path(&rel);
            let tx = if classification == PathKind::Prio {
                prio_tx.clone()
            } else {
                secondary_tx.clone()
            };

            let deduplicator = if classification == PathKind::Prio {
                &mut prio_deduplicator
            } else {
                &mut secondary_deduplicator
            };

            if deduplicator.insert(hash.clone()) {
                tx.send(ZipWriteCmd::File {
                    name: hash.clone(),
                    data: bytes,
                })
                .map_err(|e| anyhow!("ZIP writer channel closed: {}", e))?;
            }

            if classification == PathKind::Prio {
                manifest.prio.insert(
                    rel.clone(),
                    ManifestEntry {
                        store: StoreKind::Meta,
                        rel_path: rel,
                        size,
                        blake3: hash,
                    },
                );
            } else {
                manifest.secondary.insert(
                    rel.clone(),
                    ManifestEntry {
                        store: StoreKind::Meta,
                        rel_path: rel,
                        size,
                        blake3: hash,
                    },
                );
            }
        }

        for (path, size) in storage_files {
            let full = path.to_string();
            let rel = full.strip_prefix(&base_prefix).unwrap_or(&full).to_string();
            let bytes = read_store_file_bytes(storage.clone(), &path).await?;
            let hash = blake3_hash(&bytes);

            let classification = classify_path(&rel);
            let tx = if classification == PathKind::Prio {
                prio_tx.clone()
            } else {
                secondary_tx.clone()
            };

            let deduplicator = if classification == PathKind::Prio {
                &mut prio_deduplicator
            } else {
                &mut secondary_deduplicator
            };

            if deduplicator.insert(hash.clone()) {
                tx.send(ZipWriteCmd::File {
                    name: hash.clone(),
                    data: bytes,
                })
                .map_err(|e| anyhow!("ZIP writer channel closed: {}", e))?;
            }

            if classification == PathKind::Prio {
                manifest.prio.insert(
                    rel.clone(),
                    ManifestEntry {
                        store: StoreKind::Storage,
                        rel_path: rel,
                        size,
                        blake3: hash,
                    },
                );
            } else {
                manifest.secondary.insert(
                    rel.clone(),
                    ManifestEntry {
                        store: StoreKind::Storage,
                        rel_path: rel,
                        size,
                        blake3: hash,
                    },
                );
            }
        }

        // Finalize ZIP files
        prio_tx.send(ZipWriteCmd::Done).ok();
        secondary_tx.send(ZipWriteCmd::Done).ok();
        prio_handle
            .await
            .map_err(|e| anyhow!("ZIP writer task failed: {}", e))??;
        secondary_handle
            .await
            .map_err(|e| anyhow!("ZIP writer task failed: {}", e))??;

        let manifest_bytes = flow_like_types::json::to_vec(&manifest)?;
        manifest_tx
            .send(ZipWriteCmd::File {
                name: "manifest.json".to_string(),
                data: manifest_bytes,
            })
            .map_err(|e| anyhow!("ZIP writer channel closed: {}", e))?;

        manifest_tx.send(ZipWriteCmd::Done).ok();
        manifest_handle
            .await
            .map_err(|e| anyhow!("ZIP writer task failed: {}", e))??;

        let mut trailer = TrailerV1::new();

        prio_zip_clone.seek(SeekFrom::Start(0))?;
        secondary_zip_clone.seek(SeekFrom::Start(0))?;
        manifest_file_clone.seek(SeekFrom::Start(0))?;

        let mut file = File::create(&target_file)?;
        if let Some(pw) = password {
            // Encrypt all three segments first so we can compute a binding tag.
            let mut manifest_plain = Vec::new();
            manifest_file_clone.read_to_end(&mut manifest_plain)?;
            let enc_manifest = encrypt_bytes(&pw, &manifest_plain)?;
            trailer.manifest_size = enc_manifest.len() as u64;

            let mut prio_plain = Vec::new();
            prio_zip_clone.read_to_end(&mut prio_plain)?;
            let enc_prio = encrypt_bytes(&pw, &prio_plain)?;
            trailer.prio_size = enc_prio.len() as u64;

            let mut secondary_plain = Vec::new();
            secondary_zip_clone.read_to_end(&mut secondary_plain)?;
            let enc_secondary = encrypt_bytes(&pw, &secondary_plain)?;
            trailer.secondary_size = enc_secondary.len() as u64;

            let trailer_bytes = trailer.to_bytes();

            // --- Compute archive-level binding tag ---
            use flow_like_types::rand::rngs::OsRng;
            let mut binding_salt = [0u8; BINDING_SALT_LEN];
            OsRng.try_fill_bytes(&mut binding_salt)?;
            let binding_key = derive_binding_key(&pw, &binding_salt)?;
            let binding_tag = compute_binding_tag(
                &binding_key,
                &trailer_bytes,
                &enc_manifest,
                &enc_prio,
                &enc_secondary,
            );

            // Layout: [enc_manifest][enc_prio][enc_secondary][trailer][binding_salt][binding_tag]
            file.write_all(&enc_manifest)?;
            file.write_all(&enc_prio)?;
            file.write_all(&enc_secondary)?;
            file.write_all(&trailer_bytes)?;
            file.write_all(&binding_salt)?;
            file.write_all(&binding_tag)?;
            file.flush()?;
        } else {
            {
                let mut manifest_bytes = Vec::new();
                manifest_file_clone.read_to_end(&mut manifest_bytes)?;
                file.write_all(&manifest_bytes)?;
                trailer.manifest_size = manifest_bytes.len() as u64;
            }

            {
                let mut prio_bytes = Vec::new();
                prio_zip_clone.read_to_end(&mut prio_bytes)?;
                file.write_all(&prio_bytes)?;
                trailer.prio_size = prio_bytes.len() as u64;
            }

            {
                let mut secondary_bytes = Vec::new();
                secondary_zip_clone.read_to_end(&mut secondary_bytes)?;
                file.write_all(&secondary_bytes)?;
                trailer.secondary_size = secondary_bytes.len() as u64;
            }
            let trailer_bytes = trailer.to_bytes();
            file.write_all(&trailer_bytes)?;
            file.flush()?;
        }

        Ok(target_file)
    }

    pub async fn import_archive(
        app_state: Arc<FlowLikeState>,
        source_file: PathBuf,
        password: Option<String>,
    ) -> flow_like_types::Result<Self> {
        use std::io::{Read as _, Seek as _};
        use std::{fs::File, io::Cursor};

        // ---------- helpers ----------
        fn read_slice(f: &mut File, start: u64, len: u64) -> flow_like_types::Result<Vec<u8>> {
            f.seek(SeekFrom::Start(start))?;
            let mut buf = vec![0u8; len as usize];
            f.read_exact(&mut buf)?;
            Ok(buf)
        }

        fn read_manifest_from_zip(bytes: &[u8]) -> flow_like_types::Result<ExportManifest> {
            let mut zip = ZipArchive::new(Cursor::new(bytes))?;
            let mut f = zip
                .by_name("manifest.json")
                .map_err(|_| anyhow!("Missing manifest.json"))?;
            let mut buf = Vec::with_capacity(f.size() as usize);
            std::io::copy(&mut f, &mut buf)?;
            Ok(flow_like_types::json::from_slice(&buf)?)
        }

        fn join_rel_path(base: &Path, rel: &str) -> Path {
            let mut p = base.clone();
            for seg in rel.split('/') {
                if seg.is_empty() {
                    continue;
                }
                p = p.child(seg);
            }
            p
        }

        async fn load_existing_hash_and_size(
            store: &Arc<dyn ObjectStore>,
            path: &Path,
            existing_size: u64,
        ) -> flow_like_types::Result<Option<(String, u64)>> {
            if obj_exists_with_size(store, path, existing_size)
                .await?
                .unwrap_or(false)
            {
                let hash = stream_blake3_of_object(store, path).await?;
                return Ok(Some((hash, existing_size)));
            }
            Ok(None)
        }

        enum Layout {
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

        // ---------- detect & read layout (plaintext OR encrypted+bound) ----------
        let layout = task::spawn_blocking({
            let source_file = source_file.clone();
            move || -> flow_like_types::Result<Layout> {
                let mut f = File::open(&source_file)?;
                let total = f.metadata()?.len();
                if total < 24 {
                    return Err(anyhow!("file too small"));
                }

                // Try encrypted+bound first: trailer before [salt(16)+tag(32)]
                const TAIL_LEN: i64 = (BINDING_SALT_LEN + BINDING_TAG_LEN) as i64; // 48
                if total >= 24 + (TAIL_LEN as u64) {
                    let mut t_enc = [0u8; 24];
                    f.seek(SeekFrom::End(-(24 + TAIL_LEN)))?;
                    f.read_exact(&mut t_enc)?;
                    let trailer_enc = TrailerV1::from_bytes(&t_enc);
                    let m = trailer_enc.manifest_size;
                    let p = trailer_enc.prio_size;
                    let s = trailer_enc.secondary_size;
                    let header_len = m
                        .checked_add(p)
                        .ok_or_else(|| anyhow!("overflow"))?
                        .checked_add(s)
                        .ok_or_else(|| anyhow!("overflow"))?;
                    let expected_total_enc = header_len
                        .checked_add(24 + (TAIL_LEN as u64))
                        .ok_or_else(|| anyhow!("overflow"))?;
                    if expected_total_enc == total {
                        // Read segments and tail
                        let manifest_ct = read_slice(&mut f, 0, m)?;
                        let prio_ct = read_slice(&mut f, m, p)?;
                        let secondary_ct = read_slice(&mut f, m + p, s)?;
                        f.seek(SeekFrom::End(-TAIL_LEN))?;
                        let mut salt = [0u8; BINDING_SALT_LEN];
                        let mut tag = [0u8; BINDING_TAG_LEN];
                        f.read_exact(&mut salt)?;
                        f.read_exact(&mut tag)?;
                        return Ok(Layout::Encrypted {
                            trailer: trailer_enc,
                            binding_salt: salt,
                            binding_tag: tag,
                            manifest_ct,
                            prio_ct,
                            secondary_ct,
                        });
                    }
                }

                // Fallback to plaintext: trailer at EOF-24 (must not be encrypted!)
                let mut t_plain = [0u8; 24];
                f.seek(SeekFrom::End(-24))?;
                f.read_exact(&mut t_plain)?;
                let trailer_plain = TrailerV1::from_bytes(&t_plain);
                let m = trailer_plain.manifest_size;
                let p = trailer_plain.prio_size;
                let s = trailer_plain.secondary_size;
                let header_len = m
                    .checked_add(p)
                    .ok_or_else(|| anyhow!("overflow"))?
                    .checked_add(s)
                    .ok_or_else(|| anyhow!("overflow"))?;
                let expected_total_plain = header_len
                    .checked_add(24)
                    .ok_or_else(|| anyhow!("overflow"))?;
                if expected_total_plain != total {
                    return Err(anyhow!("trailer sizes don't match file length"));
                }

                let manifest = read_slice(&mut f, 0, m)?;
                let prio = read_slice(&mut f, m, p)?;
                let secondary = read_slice(&mut f, m + p, s)?;

                // If any segment looks encrypted, reject (we don't accept old encrypted w/o tag)
                let looks_enc = |seg: &Vec<u8>| {
                    seg.len() >= ENC_MAGIC.len() && &seg[..ENC_MAGIC.len()] == ENC_MAGIC
                };
                if looks_enc(&manifest) || looks_enc(&prio) || looks_enc(&secondary) {
                    return Err(anyhow!(
                        "Encrypted archive missing binding tag (unsupported legacy format)"
                    ));
                }

                Ok(Layout::Plain {
                    manifest,
                    prio,
                    secondary,
                })
            }
        })
        .await
        .map_err(|e| anyhow!("I/O task failed: {}", e))??;

        // ---------- verify binding (if encrypted), then parse manifest ----------
        let (manifest_zip_bytes, prio_seg, secondary_seg, decrypt_on_demand) = match layout {
            Layout::Plain {
                manifest,
                prio,
                secondary,
                ..
            } => (manifest, prio, secondary, false),
            Layout::Encrypted {
                trailer,
                binding_salt,
                binding_tag,
                manifest_ct,
                prio_ct,
                secondary_ct,
            } => {
                let pw = password
                    .as_ref()
                    .ok_or_else(|| anyhow!("Password required for encrypted archive"))?;
                let binding_key = derive_binding_key(pw, &binding_salt)?;
                let trailer_bytes = trailer.to_bytes();
                let expected = compute_binding_tag(
                    &binding_key,
                    &trailer_bytes,
                    &manifest_ct,
                    &prio_ct,
                    &secondary_ct,
                );
                if expected != binding_tag {
                    bail!(
                        "Archive authentication failed (binding tag mismatch), {:?}, {:?}",
                        expected,
                        binding_tag
                    );
                }
                // Decrypt manifest only (cheap)
                let manifest_zip_bytes = task::spawn_blocking({
                    let pw = password.clone();
                    move || decrypt_bytes(pw.as_ref().unwrap(), &manifest_ct)
                })
                .await
                .map_err(|e| anyhow!("Decrypt task failed: {}", e))??;

                (manifest_zip_bytes, prio_ct, secondary_ct, true)
            }
        };

        // Parse manifest
        let manifest: ExportManifest = task::spawn_blocking({
            let m = manifest_zip_bytes.clone();
            move || read_manifest_from_zip(&m)
        })
        .await
        .map_err(|e| anyhow!("Manifest read task failed: {}", e))??;

        let meta = FlowLikeState::project_meta_store(&app_state)
            .await?
            .as_generic();
        let storage = FlowLikeState::project_storage_store(&app_state)
            .await?
            .as_generic();
        let base = Path::from("apps").child(manifest.app_id.clone());

        #[derive(Default)]
        struct Plan {
            prio: HashMap<String, Vec<String>>,      // blob hash -> rel paths
            secondary: HashMap<String, Vec<String>>, // blob hash -> rel paths
        }
        let mut plan = Plan::default();

        async fn plan_map(
            entries: &HashMap<String, ManifestEntry>,
            out: &mut HashMap<String, Vec<String>>,
            meta: &Arc<dyn ObjectStore>,
            storage: &Arc<dyn ObjectStore>,
            base: &Path,
        ) -> flow_like_types::Result<()> {
            for (rel, e) in entries {
                let store = match e.store {
                    StoreKind::Meta => meta,
                    StoreKind::Storage => storage,
                };
                let target = join_rel_path(base, rel);
                match load_existing_hash_and_size(store, &target, e.size).await? {
                    None => out.entry(e.blake3.clone()).or_default().push(rel.clone()),
                    Some((existing_hash, _)) => {
                        if existing_hash != e.blake3 {
                            out.entry(e.blake3.clone()).or_default().push(rel.clone());
                        }
                    }
                }
            }
            Ok(())
        }

        plan_map(&manifest.prio, &mut plan.prio, &meta, &storage, &base).await?;
        plan_map(
            &manifest.secondary,
            &mut plan.secondary,
            &meta,
            &storage,
            &base,
        )
        .await?;

        if plan.prio.is_empty() && plan.secondary.is_empty() {
            let mut app = App::load(manifest.app_id.clone(), app_state.clone()).await?;
            app.updated_at = SystemTime::now();
            app.save().await?;
            return Ok(app);
        }

        // Prepare ZIP archives for restore; decrypt prio/secondary only if needed and encrypted.
        let (mut prio_zip_opt, mut secondary_zip_opt) = if decrypt_on_demand {
            let (prio_plain_opt, secondary_plain_opt) = task::spawn_blocking({
                let pw = password.clone();
                let prio_seg = prio_seg.clone();
                let secondary_seg = secondary_seg.clone();
                let need_prio = !plan.prio.is_empty();
                let need_secondary = !plan.secondary.is_empty();
                move || -> flow_like_types::Result<(Option<Vec<u8>>, Option<Vec<u8>>)> {
                    let p = if need_prio {
                        Some(decrypt_bytes(pw.as_ref().unwrap(), &prio_seg)?)
                    } else {
                        None
                    };
                    let s = if need_secondary {
                        Some(decrypt_bytes(pw.as_ref().unwrap(), &secondary_seg)?)
                    } else {
                        None
                    };
                    Ok((p, s))
                }
            })
            .await
            .map_err(|e| anyhow!("Decrypt task failed: {}", e))??;

            let prio_zip = match prio_plain_opt {
                Some(p) => Some(ZipArchive::new(Cursor::new(p))?),
                None => None,
            };
            let secondary_zip = match secondary_plain_opt {
                Some(s) => Some(ZipArchive::new(Cursor::new(s))?),
                None => None,
            };
            (prio_zip, secondary_zip)
        } else {
            // Plaintext case
            let prio_zip = if !plan.prio.is_empty() {
                Some(ZipArchive::new(Cursor::new(prio_seg))?)
            } else {
                None
            };
            let secondary_zip = if !plan.secondary.is_empty() {
                Some(ZipArchive::new(Cursor::new(secondary_seg))?)
            } else {
                None
            };
            (prio_zip, secondary_zip)
        };

        let mut blob_cache: HashMap<String, Bytes> = HashMap::new();

        fn extract_blob(
            arc: &mut ZipArchive<Cursor<Vec<u8>>>,
            hash: &str,
        ) -> flow_like_types::Result<Vec<u8>> {
            let mut f = arc
                .by_name(hash)
                .map_err(|_| anyhow!("Missing blob {}", hash))?;
            let mut buf = Vec::with_capacity(f.size() as usize);
            std::io::copy(&mut f, &mut buf)?;
            Ok(buf)
        }

        async fn restore_set(
            arc: &mut ZipArchive<Cursor<Vec<u8>>>,
            items: &HashMap<String, Vec<String>>, // blob -> rels
            entries: &HashMap<String, ManifestEntry>,
            base: &Path,
            meta: &Arc<dyn ObjectStore>,
            storage: &Arc<dyn ObjectStore>,
            blob_cache: &mut HashMap<String, Bytes>,
        ) -> flow_like_types::Result<()> {
            for (blob, rels) in items {
                let data_bytes: Bytes = if let Some(b) = blob_cache.get(blob) {
                    b.clone()
                } else {
                    let raw = extract_blob(arc, blob)?;
                    let b = Bytes::from(raw);
                    blob_cache.insert(blob.clone(), b.clone());
                    b
                };
                for rel in rels {
                    let e = entries
                        .get(rel)
                        .ok_or_else(|| anyhow!("Missing manifest entry for {}", rel))?;
                    if (data_bytes.len() as u64) != e.size || blake3_hash(&data_bytes) != e.blake3 {
                        return Err(anyhow!("Integrity failure for {}", rel));
                    }
                    let s = match e.store {
                        StoreKind::Meta => meta,
                        StoreKind::Storage => storage,
                    };
                    let target = join_rel_path(base, rel);
                    s.put(&target, PutPayload::from_bytes(data_bytes.clone()))
                        .await?;
                }
            }
            Ok(())
        }

        if let Some(ref mut arc) = prio_zip_opt {
            restore_set(
                arc,
                &plan.prio,
                &manifest.prio,
                &base,
                &meta,
                &storage,
                &mut blob_cache,
            )
            .await?;
        }
        if let Some(ref mut arc) = secondary_zip_opt {
            restore_set(
                arc,
                &plan.secondary,
                &manifest.secondary,
                &base,
                &meta,
                &storage,
                &mut blob_cache,
            )
            .await?;
        }

        let mut app = App::load(manifest.app_id.clone(), app_state.clone()).await?;
        app.updated_at = SystemTime::now();
        app.save().await?;
        Ok(app)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        app::{AppExecutionMode, AppStatus, AppVisibility},
        state::FlowLikeConfig,
        utils::http::HTTPClient,
    };
    use flow_like_storage::{
        files::store::FlowLikeStore,
        object_store::{ObjectStore, PutPayload, memory::InMemory, path::Path as ObjectPath},
    };
    use flow_like_types::tokio;
    use std::{fs::File, io::Read, sync::Arc, time::SystemTime};
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
        let meta_path = ObjectPath::from("apps").child(app_id).child("config.meta");
        meta.put(
            &meta_path,
            PutPayload::from_bytes(Bytes::from_static(b"meta config")),
        )
        .await
        .expect("seed meta");

        let storage_path = ObjectPath::from("apps").child(app_id).child("data.bin");
        storage
            .put(
                &storage_path,
                PutPayload::from_bytes(Bytes::from_static(b"storage data")),
            )
            .await
            .expect("seed blob");
    }

    // ---------- Encryption tests ----------

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
        assert_ne!(enc, plain, "ciphertext must differ from plaintext");
        assert!(
            enc.starts_with(ENC_MAGIC),
            "ciphertext must start with ENC_MAGIC"
        );

        // Structure: MAGIC + salt + nonce + ciphertext
        assert!(
            enc.len() > ENC_MAGIC.len() + SALT_LEN + XNONCE_LEN,
            "has room for ciphertext"
        );

        // Different encryptions should be different (random salt/nonce)
        let enc2 = encrypt_bytes(pw, plain).expect("encrypt");
        assert_ne!(enc, enc2, "randomized encryption should differ per call");

        let dec = decrypt_bytes(pw, &enc).expect("decrypt");
        assert_eq!(dec, plain);
    }

    #[test]
    fn decrypt_with_wrong_password_fails() {
        let enc = encrypt_bytes("pw1", b"top secret").expect("encrypt");
        let err = decrypt_bytes("pw2", &enc).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.to_lowercase().contains("decrypt") || msg.to_lowercase().contains("failed"),
            "expected decrypt error, got: {msg}"
        );
    }

    #[test]
    fn decrypt_with_wrong_magic_fails() {
        let pw = "pw";
        let mut bad = encrypt_bytes(pw, b"abc").expect("encrypt");
        // Corrupt the magic
        bad[0] ^= 0xFF;
        let err = decrypt_bytes(pw, &bad).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.to_lowercase()
                .contains("invalid encrypted archive header"),
            "expected header error, got: {msg}"
        );
    }

    #[test]
    fn encryption_output_is_not_zip() {
        let pw = "pw";
        let enc = encrypt_bytes(pw, b"some payload").expect("encrypt");
        assert!(!is_zip(&enc), "encrypted blob must not be detected as zip");
    }

    // ---------- ZIP writer tests ----------

    #[test]
    fn zip_write_streaming_writes_files_and_content() {
        // Prepare a temp target
        let tmp = NamedTempFile::new().expect("tmp");
        let target_path = tmp.path().to_path_buf();

        let (tx, rx) = std::sync::mpsc::channel::<ZipWriteCmd>();

        // Write in a blocking helper thread (as production code does)
        let target_file = File::create(&target_path).expect("create target file");
        let handle = std::thread::spawn(move || zip_write_streaming(target_file, rx));

        // Send two small files and finish
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

        // Re-open the written zip and verify contents
        let mut f = File::open(tmp.path()).expect("open zip");
        let mut zip = ZipArchive::new(&mut f).expect("read zip");

        // dir/one.txt
        {
            let mut file = zip.by_name("dir/one.txt").expect("missing one.txt");
            assert_eq!(file.name(), "dir/one.txt");
            let mut buf = String::new();
            file.read_to_string(&mut buf).expect("read one.txt");
            assert_eq!(buf, "first");
        }

        // two.bin
        {
            let mut file = zip.by_name("two.bin").expect("missing two.bin");
            assert_eq!(file.name(), "two.bin");
            let mut buf = Vec::new();
            file.read_to_end(&mut buf).expect("read two.bin");
            assert_eq!(buf, vec![1, 2, 3, 4, 5]);
        }
    }

    #[test]
    fn is_zip_detects_zip_headers() {
        // Minimal PK header detection test using a real tiny zip written by the same helper
        let tmp = NamedTempFile::new().expect("tmp");
        let (tx, rx) = std::sync::mpsc::channel::<ZipWriteCmd>();
        let path = tmp.path().to_path_buf();
        let target_file = File::create(&path).expect("create target file");
        let h = std::thread::spawn(move || zip_write_streaming(target_file, rx));
        tx.send(ZipWriteCmd::File {
            name: "x".into(),
            data: b"y".to_vec(),
        })
        .unwrap();
        tx.send(ZipWriteCmd::Done).unwrap();
        h.join().unwrap().unwrap();

        // Read first 8 bytes and call is_zip
        let header = {
            let mut f = File::open(tmp.path()).unwrap();
            let mut buf = [0u8; 8];
            let n = f.read(&mut buf).unwrap();
            buf[..n].to_vec()
        };
        assert!(is_zip(&header), "should detect PK header");
    }

    #[test]
    fn zip_encrypt_decrypt_roundtrip_integrated() {
        use std::{
            fs::File,
            io::{Cursor, Read},
        };
        use tempfile::NamedTempFile;

        // 1) Build a tiny ZIP via the streaming writer
        let tmp = NamedTempFile::new().expect("tmp");
        let target_path = tmp.path().to_path_buf();
        let (tx, rx) = std::sync::mpsc::channel::<ZipWriteCmd>();
        let target_file = File::create(&target_path).expect("create target file");
        let writer = std::thread::spawn(move || zip_write_streaming(target_file, rx));

        tx.send(ZipWriteCmd::File {
            name: "dir/one.txt".into(),
            data: b"first".to_vec(),
        })
        .unwrap();
        tx.send(ZipWriteCmd::File {
            name: "two.bin".into(),
            data: vec![1, 2, 3, 4, 5],
        })
        .unwrap();
        tx.send(ZipWriteCmd::Done).unwrap();

        writer.join().unwrap().expect("zip write ok");

        // Read the produced ZIP bytes
        let mut f = File::open(tmp.path()).expect("open zip");
        let mut zip_bytes = Vec::new();
        f.read_to_end(&mut zip_bytes).expect("read zip");
        assert!(is_zip(&zip_bytes), "freshly-written bytes should be a ZIP");

        // 2) Encrypt the ZIP and ensure it is not mistaken as ZIP
        let pw = "sup3r-secret";
        let enc = encrypt_bytes(pw, &zip_bytes).expect("encrypt");
        assert!(!is_zip(&enc), "encrypted blob must not be detected as ZIP");
        assert!(
            enc.starts_with(ENC_MAGIC),
            "encrypted blob must start with magic"
        );

        // 3) Decrypt and check exact byte-for-byte equality
        let dec = decrypt_bytes(pw, &enc).expect("decrypt");
        assert_eq!(dec, zip_bytes, "decrypt must restore original ZIP bytes");
        assert!(is_zip(&dec), "decrypted bytes must be a ZIP again");

        // 4) Open decrypted ZIP and verify contents
        let cursor = Cursor::new(dec);
        let mut zip = ZipArchive::new(cursor).expect("valid zip after decrypt");

        // dir/one.txt
        {
            let mut file = zip.by_name("dir/one.txt").expect("one.txt present");
            let mut buf = String::new();
            use std::io::Read as _;
            file.read_to_string(&mut buf).expect("read one.txt");
            assert_eq!(buf, "first");
        }

        // two.bin
        {
            let mut file = zip.by_name("two.bin").expect("two.bin present");
            let mut buf = Vec::new();
            use std::io::Read as _;
            file.read_to_end(&mut buf).expect("read two.bin");
            assert_eq!(buf, vec![1, 2, 3, 4, 5]);
        }
    }

    #[test]
    fn zip_encrypt_decrypt_big_file() {
        use flow_like_types::rand::{RngCore, SeedableRng, rngs::StdRng};
        use std::{
            fs::File,
            io::{Cursor, Read},
        };
        use tempfile::NamedTempFile;

        // 1) Create a 100MB pseudo-random file in memory
        let mut big_data = vec![0u8; 100 * 1024 * 1024];
        let mut rng = StdRng::seed_from_u64(42); // deterministic
        rng.fill_bytes(&mut big_data);

        // 2) Stream ZIP write
        let tmp = NamedTempFile::new().expect("tmp");
        let target_path = tmp.path().to_path_buf();
        let (tx, rx) = std::sync::mpsc::channel::<ZipWriteCmd>();
        let target_file = File::create(&target_path).expect("create target file");
        let writer = std::thread::spawn(move || zip_write_streaming(target_file, rx));

        tx.send(ZipWriteCmd::File {
            name: "big.bin".into(),
            data: big_data.clone(),
        })
        .unwrap();
        tx.send(ZipWriteCmd::Done).unwrap();
        writer.join().unwrap().expect("zip write ok");

        // 3) Encrypt & decrypt
        let mut zip_bytes = Vec::new();
        File::open(tmp.path())
            .unwrap()
            .read_to_end(&mut zip_bytes)
            .unwrap();
        let pw = "bigpass";
        let enc = encrypt_bytes(pw, &zip_bytes).expect("encrypt");
        let dec = decrypt_bytes(pw, &enc).expect("decrypt");
        assert_eq!(zip_bytes, dec);

        // 4) Verify ZIP content
        let mut zip = ZipArchive::new(Cursor::new(dec)).expect("valid zip");
        let mut file = zip.by_name("big.bin").expect("missing big.bin");
        let mut extracted = Vec::new();
        file.read_to_end(&mut extracted).unwrap();
        assert_eq!(extracted, big_data);
    }

    #[test]
    fn export_import_roundtrip_plaintext_archive() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        rt.block_on(async {
            let (state, meta_store, storage_store) = setup_state();
            let app_id = "app_plain";

            let app = make_app(app_id, state.clone());
            app.save().await.expect("seed manifest");
            seed_sample_objects(&meta_store, &storage_store, app_id).await;

            let tempdir = tempfile::tempdir().expect("tmpdir");
            let export_target = tempdir.path().join("plain_archive");

            let exported_path = app
                .export_archive(None, export_target)
                .await
                .expect("export plain");

            let (import_state, import_meta, import_storage) = setup_state();
            let imported = App::import_archive(import_state, exported_path.clone(), None)
                .await
                .expect("import plain");
            assert_eq!(imported.id, app_id);

            let manifest_path = ObjectPath::from("apps").child(app_id).child("manifest.app");
            let manifest_bytes = import_meta
                .get(&manifest_path)
                .await
                .expect("fetch manifest")
                .bytes()
                .await
                .expect("manifest bytes");
            assert!(!manifest_bytes.is_empty(), "manifest should not be empty");

            let config_path = ObjectPath::from("apps").child(app_id).child("config.meta");
            let config_bytes = import_meta
                .get(&config_path)
                .await
                .expect("fetch config meta")
                .bytes()
                .await
                .expect("config bytes");
            assert_eq!(config_bytes, Bytes::from_static(b"meta config"));

            let storage_path = ObjectPath::from("apps").child(app_id).child("data.bin");
            let blob_bytes = import_storage
                .get(&storage_path)
                .await
                .expect("fetch blob")
                .bytes()
                .await
                .expect("blob bytes");
            assert_eq!(blob_bytes, Bytes::from_static(b"storage data"));
        });
    }

    #[test]
    fn export_import_roundtrip_encrypted_archive() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        rt.block_on(async {
            let (state, meta_store, storage_store) = setup_state();
            let app_id = "app_enc";

            let app = make_app(app_id, state.clone());
            app.save().await.expect("seed manifest");
            seed_sample_objects(&meta_store, &storage_store, app_id).await;

            let tempdir = tempfile::tempdir().expect("tmpdir");
            let export_target = tempdir.path().join("encrypted_archive");
            let password = "s3cret-pass";

            let exported_path = app
                .export_archive(Some(password.to_string()), export_target)
                .await
                .expect("export enc");

            let (import_state, import_meta, import_storage) = setup_state();
            let imported = App::import_archive(
                import_state,
                exported_path.clone(),
                Some(password.to_string()),
            )
            .await
            .expect("import enc");
            assert_eq!(imported.id, app_id);

            let manifest_path = ObjectPath::from("apps").child(app_id).child("manifest.app");
            let manifest_bytes = import_meta
                .get(&manifest_path)
                .await
                .expect("fetch manifest")
                .bytes()
                .await
                .expect("manifest bytes");
            assert!(!manifest_bytes.is_empty(), "manifest should not be empty");

            let config_path = ObjectPath::from("apps").child(app_id).child("config.meta");
            let config_bytes = import_meta
                .get(&config_path)
                .await
                .expect("fetch config meta")
                .bytes()
                .await
                .expect("config bytes");
            assert_eq!(config_bytes, Bytes::from_static(b"meta config"));

            let storage_path = ObjectPath::from("apps").child(app_id).child("data.bin");
            let blob_bytes = import_storage
                .get(&storage_path)
                .await
                .expect("fetch blob")
                .bytes()
                .await
                .expect("blob bytes");
            assert_eq!(blob_bytes, Bytes::from_static(b"storage data"));
        });
    }
}
