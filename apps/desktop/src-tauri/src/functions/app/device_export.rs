use crate::{
    functions::TauriFunctionError,
    state::{TauriFlowLikeState, TauriRegistryState, TauriSettingsState},
};
use anyhow::{Context, Result, ensure};
use flow_like::{
    app::{
        App,
        sharing::device::{DeviceExportFile, DeviceProjectSnapshot, MAX_DEVICE_EXPORT_CHUNK},
    },
    flow_like_storage::{Path, object_store::ObjectStoreExt},
    state::FlowLikeState,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, LazyLock},
    time::{Duration, Instant},
};
use tauri::AppHandle;
use tokio::sync::{Mutex, Semaphore};

const TTL: Duration = Duration::from_secs(3600);
static PREPARING: Semaphore = Semaphore::const_new(1);
static EXPORTS: LazyLock<Mutex<HashMap<String, ExportSession>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

struct ExportSession {
    owner: String,
    expires: Instant,
    snapshot: Arc<DeviceProjectSnapshot>,
}

#[derive(Clone, Default, Serialize)]
pub struct Assets {
    bit_pins: Vec<BitPin>,
    package_pins: Vec<PackagePin>,
}
#[derive(Clone, Serialize)]
struct BitPin {
    bit_id: String,
    metadata_sha256: String,
}
#[derive(Clone, Serialize)]
struct PackagePin {
    package_id: String,
    version: String,
    wasm_sha256: String,
    manifest_sha256: String,
}
#[derive(Serialize)]
pub struct PreparedExport {
    export_id: String,
    project_id: String,
    source: &'static str,
    files: Vec<DeviceExportFile>,
    assets: Assets,
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn snapshot_digest(snapshot: &DeviceProjectSnapshot, path: &str, size: u64) -> Result<String> {
    let mut hash = Sha256::new();
    let mut offset = 0;
    while offset < size {
        let length = (size - offset).min(MAX_DEVICE_EXPORT_CHUNK as u64) as usize;
        hash.update(snapshot.read_chunk(path, offset, length)?);
        offset += length as u64;
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn identifier(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value.len() <= 128
            && value != "."
            && value != ".."
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.')),
        "Invalid dependency identifier"
    );
    Ok(())
}
fn reject_embedded_credentials(value: &serde_json::Value) -> Result<()> {
    match value {
        serde_json::Value::Object(values) => {
            for (key, value) in values {
                if matches!(
                    key.to_ascii_lowercase().as_str(),
                    "api_key"
                        | "apikey"
                        | "access_token"
                        | "refresh_token"
                        | "password"
                        | "secret"
                        | "authorization"
                ) {
                    ensure!(
                        value.is_null() || value.as_str() == Some(""),
                        "A selected model contains embedded credentials. Configure credentials separately on the device."
                    );
                }
                reject_embedded_credentials(value)?;
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                reject_embedded_credentials(value)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn private_metadata_url(value: &serde_json::Value) -> bool {
    value
        .as_str()
        .and_then(|text| reqwest::Url::parse(text).ok())
        .is_some_and(|url| {
            url.query().is_some()
                || url.fragment().is_some()
                || !url.username().is_empty()
                || url.password().is_some()
        })
}

fn public_metadata(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(values) => {
            values.retain(|key, value| {
                !matches!(
                    key.as_str(),
                    "download_link"
                        | "download_url"
                        | "cwasm_download_url"
                        | "widget_bundle_download_url"
                ) && !private_metadata_url(value)
            });
            for value in values.values_mut() {
                public_metadata(value);
            }
        }
        serde_json::Value::Array(values) => {
            values.retain(|value| !private_metadata_url(value));
            for value in values {
                public_metadata(value);
            }
        }
        _ => {}
    }
}

fn public_bit_metadata(bit: &flow_like::bit::Bit) -> Result<serde_json::Value> {
    reject_embedded_credentials(&bit.parameters)?;
    let inline = |bit: &flow_like::bit::Bit| -> Result<Vec<serde_json::Value>> {
        let mut assets = bit.inline_mlx_asset_bits()
            .map_err(|_| anyhow::anyhow!("Inline MLX assets need a pinned repository revision and complete file manifest before deployment"))?;
        assets.extend(bit.projection_bit());
        Ok(assets
            .into_iter()
            .map(|asset| {
                serde_json::json!({
                    "id":asset.id,"hash":asset.hash,"file_name":asset.file_name,"size":asset.size,
                })
            })
            .collect())
    };
    let before = inline(bit)?;
    let mut value = serde_json::to_value(bit)?;
    public_metadata(&mut value);
    if let Some(projection) = bit.projection_bit() {
        let source = projection
            .download_link
            .context("Inline projector source is missing")?;
        let url = reqwest::Url::parse(&source).map_err(|_| {
            anyhow::anyhow!(
                "Inline projector needs a public canonical HTTPS source before deployment"
            )
        })?;
        ensure!(
            url.scheme() == "https"
                && url.as_str() == source
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none(),
            "Inline projector uses a signed or unsafe source URL. Select a public canonical HTTPS source without credentials, query parameters or fragments before deployment."
        );
        // This URL is part of the immutable source-derived identity, not a
        // download fallback. The artifact separately pins the copied bytes.
        let parameters = value
            .pointer_mut("/parameters/provider/params/projection")
            .and_then(serde_json::Value::as_object_mut)
            .context("Inline projector metadata changed during deployment preparation")?;
        parameters.insert("download_link".into(), serde_json::Value::String(source));
    }
    let public: flow_like::bit::Bit = serde_json::from_value(value.clone())?;
    ensure!(
        inline(&public)? == before,
        "Inline model asset identity changed when private metadata was removed. Select public pinned model sources before deployment."
    );
    Ok(value)
}

async fn dependencies(
    handle: &AppHandle,
    app: &App,
    snapshot: &mut DeviceProjectSnapshot,
) -> Result<Assets> {
    ensure!(
        app.bits.len() <= 256 && app.packages.len() <= 64,
        "Too many selected dependencies"
    );
    let state = TauriFlowLikeState::construct(handle).await?;
    let profile = TauriSettingsState::current_profile(handle).await?;
    let http = TauriFlowLikeState::http_client(handle).await?;
    let bits_source = FlowLikeState::bit_store(&state).await?;
    let bits_store = bits_source.as_generic();
    let mut assets = Assets::default();
    let mut files = BTreeMap::<String, (u64, String)>::new();
    for reference in &app.bits {
        let (hub, id) = reference
            .rsplit_once(':')
            .map_or((None, reference.as_str()), |(hub, id)| {
                (Some(hub.to_owned()), id)
            });
        identifier(id)?;
        let bit = profile
            .hub_profile
            .get_bit(id.to_owned(), hub, http.clone())
            .await?;
        let pack = bit.pack(state.clone()).await?;
        let public_bit = public_bit_metadata(&bit)?;
        let public_dependencies = pack
            .bits
            .iter()
            .filter(|item| item.id != bit.id)
            .map(public_bit_metadata)
            .collect::<Result<Vec<_>>>()?;
        ensure!(pack.bits.len() <= 2049, "Too many model dependency files");
        let selected_bytes = pack
            .bits
            .iter()
            .filter(|item| item.file_name.is_some())
            .try_fold(0_u64, |total, item| {
                let size = item.size.context(
                    "Model artifact size is unknown; refresh its metadata before deployment",
                )?;
                ensure!(
                    size > 0 && size <= 4 * 1024 * 1024 * 1024,
                    "Model artifact exceeds 4 GiB"
                );
                total.checked_add(size).context("Model asset size overflow")
            })?;
        ensure!(
            selected_bytes <= 8 * 1024 * 1024 * 1024,
            "Selected model assets exceed 8 GiB"
        );
        if !pack.is_installed(state.clone()).await? {
            pack.download(state.clone(), None)
                .await
                .context("A required model asset could not be downloaded")?;
        }
        let mut artifacts = BTreeMap::new();
        for item in &pack.bits {
            identifier(&item.id)?;
            reject_embedded_credentials(&item.parameters)?;
            let Some(name) = &item.file_name else {
                continue;
            };
            identifier(&item.hash)?;
            ensure!(
                !name.is_empty()
                    && !name.contains('\\')
                    && name
                        .split('/')
                        .all(|p| !p.is_empty() && p != "." && p != ".."),
                "Invalid model asset filename"
            );
            let path = format!("bits/{}/{name}", item.hash);
            let (size, hash) = if let Some(existing) = files.get(&path) {
                existing.clone()
            } else {
                let location = Path::from(item.hash.as_str()).join(name.as_str());
                flow_like::app::sharing::device::validate_local_source(&bits_source, &location)?;
                let metadata = bits_store.head(&location).await.with_context(|| {
                    format!("Download model asset {name} before exporting this project")
                })?;
                ensure!(
                    item.size == Some(metadata.size),
                    "Model asset size differs from its selected metadata"
                );
                snapshot
                    .add_object(&path, bits_store.clone(), &metadata)
                    .await?;
                let hash = snapshot_digest(snapshot, &path, metadata.size)?;
                files.insert(path.clone(), (metadata.size, hash.clone()));
                (metadata.size, hash)
            };
            ensure!(
                item.size == Some(size),
                "Selected models disagree about asset size"
            );
            artifacts.insert(
                path.clone(),
                serde_json::json!({"path":path,"size":size,"sha256":hash}),
            );
        }
        let public = serde_json::json!({"bit":public_bit,"dependencies":public_dependencies,"artifacts":artifacts.into_values().collect::<Vec<_>>()});
        let bytes = serde_json::to_vec(&public)?;
        ensure!(
            bytes.len() <= 16 * 1024 * 1024,
            "Model metadata exceeds 16 MiB"
        );
        assets.bit_pins.push(BitPin {
            bit_id: id.to_owned(),
            metadata_sha256: digest(&bytes),
        });
        snapshot
            .add_bytes(&format!("bits/metadata/{id}.json"), &bytes)
            .await?;
    }
    let mut total_wasm = 0usize;
    if !app.packages.is_empty() {
        let registry = TauriRegistryState::get_client(handle).await?;
        let mut packages = app.packages.iter().collect::<Vec<_>>();
        packages.sort();
        for (id, version) in packages {
            identifier(id)?;
            identifier(version)?;
            let installed = registry.get_installed(id).await;
            let selected = installed
                .as_ref()
                .and_then(|package| package.get_version(version));
            let (manifest, wasm) = if let Some(selected) = selected {
                ensure!(
                    selected.manifest.id == *id
                        && selected.manifest.version == *version
                        && selected.manifest.validate().is_ok(),
                    "Installed package manifest differs from the project's pin"
                );
                let metadata = tokio::fs::metadata(&selected.wasm_path).await?;
                ensure!(metadata.len() <= 64 * 1024 * 1024, "Package exceeds 64 MiB");
                use tokio::io::AsyncReadExt;
                let file = tokio::fs::File::open(&selected.wasm_path).await?;
                let mut wasm = Vec::new();
                file.take(64 * 1024 * 1024 + 1)
                    .read_to_end(&mut wasm)
                    .await?;
                ensure!(
                    wasm.len() <= 64 * 1024 * 1024 && wasm.starts_with(b"\0asm"),
                    "Package must contain portable WASM bytes"
                );
                if let Some(expected) = &selected.wasm_hash {
                    ensure!(
                        expected == &blake3::hash(&wasm).to_hex().to_string(),
                        "Installed package digest differs; reinstall it before deploying"
                    );
                }
                (selected.manifest.clone(), wasm)
            } else {
                registry
                    .export_package_version(id, version, 64 * 1024 * 1024)
                    .await?
            };
            total_wasm += wasm.len();
            ensure!(
                total_wasm <= 256 * 1024 * 1024,
                "Selected WASM packages exceed 256 MiB"
            );
            let wasm_sha256 = digest(&wasm);
            let manifest = serde_json::to_vec(&manifest)?;
            ensure!(
                manifest.len() <= 1024 * 1024,
                "Package manifest exceeds 1 MiB"
            );
            snapshot
                .add_bytes(&format!("packages/{id}/{version}/module.wasm"), &wasm)
                .await?;
            snapshot
                .add_bytes(&format!("packages/{id}/{version}/manifest.json"), &manifest)
                .await?;
            assets.package_pins.push(PackagePin {
                package_id: id.clone(),
                version: version.clone(),
                wasm_sha256,
                manifest_sha256: digest(&manifest),
            });
        }
    }
    Ok(assets)
}

async fn prepare(
    handle: &AppHandle,
    app_id: String,
    online_app: Option<App>,
    user_sub: Option<String>,
) -> Result<PreparedExport> {
    let _permit = PREPARING
        .try_acquire()
        .context("Another project snapshot is being prepared")?;
    let owner = TauriSettingsState::current_profile(handle)
        .await?
        .hub_profile
        .id;
    {
        let mut sessions = EXPORTS.lock().await;
        sessions.retain(|_, session| session.expires > Instant::now());
        ensure!(
            sessions.len() < 2,
            "Finish or discard another prepared deployment before exporting"
        );
    }
    let state = TauriFlowLikeState::construct(handle).await?;
    let (app, mut snapshot, source) = if let Some(app) = online_app {
        ensure!(
            app.id == app_id && !matches!(app.visibility, flow_like::app::AppVisibility::Offline),
            "Online project identity differs"
        );
        (
            app,
            DeviceProjectSnapshot::online_dependencies(&app_id).await?,
            "online",
        )
    } else {
        let app = App::load(app_id.clone(), state).await?;
        let snapshot = app
            .export_device_snapshot(
                user_sub
                    .as_deref()
                    .context("Select the account whose local project data should be exported")?,
            )
            .await?;
        (app, snapshot, "offline")
    };
    let assets = dependencies(handle, &app, &mut snapshot).await?;
    ensure!(
        TauriSettingsState::current_profile(handle)
            .await?
            .hub_profile
            .id
            == owner,
        "The active account changed during export"
    );
    let export_id = uuid::Uuid::new_v4().to_string();
    let response = PreparedExport {
        export_id: export_id.clone(),
        project_id: app_id,
        source,
        files: snapshot.files(),
        assets,
    };
    EXPORTS.lock().await.insert(
        export_id.clone(),
        ExportSession {
            owner,
            expires: Instant::now() + TTL,
            snapshot: Arc::new(snapshot),
        },
    );
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            let mut sessions = EXPORTS.lock().await;
            let Some(session) = sessions.get(&export_id) else {
                break;
            };
            if session.expires <= Instant::now() {
                sessions.remove(&export_id);
                break;
            }
        }
    });
    Ok(response)
}

#[tauri::command]
pub async fn prepare_device_project_export(
    app_handle: AppHandle,
    app_id: String,
    online_app: Option<App>,
    user_sub: Option<String>,
) -> Result<PreparedExport, TauriFunctionError> {
    prepare(&app_handle, app_id, online_app, user_sub)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn read_device_project_export_chunk(
    app_handle: AppHandle,
    export_id: String,
    path: String,
    offset: u64,
    length: usize,
) -> Result<tauri::ipc::Response, TauriFunctionError> {
    let owner = TauriSettingsState::current_profile(&app_handle)
        .await?
        .hub_profile
        .id;
    let snapshot = {
        let mut sessions = EXPORTS.lock().await;
        let session = sessions
            .get_mut(&export_id)
            .filter(|session| session.owner == owner && session.expires > Instant::now())
            .ok_or_else(|| {
                TauriFunctionError::new("Prepared export expired or belongs to another account")
            })?;
        session.expires = Instant::now() + TTL;
        session.snapshot.clone()
    };
    let bytes = tokio::task::spawn_blocking(move || snapshot.read_chunk(&path, offset, length))
        .await
        .map_err(|error| TauriFunctionError::new(&error.to_string()))??;
    Ok(tauri::ipc::Response::new(bytes))
}

#[tauri::command]
pub async fn release_device_project_export(
    app_handle: AppHandle,
    export_id: String,
) -> Result<(), TauriFunctionError> {
    let owner = TauriSettingsState::current_profile(&app_handle)
        .await?
        .hub_profile
        .id;
    let mut sessions = EXPORTS.lock().await;
    if sessions
        .get(&export_id)
        .is_some_and(|session| session.owner == owner)
    {
        sessions.remove(&export_id);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn inline_model(provider: &str) -> flow_like::bit::Bit {
        flow_like::bit::Bit {
            id: "inline-model".into(),
            bit_type: flow_like::bit::BitTypes::Vlm,
            parameters: serde_json::json!({"context_length":8192,"model_classification":flow_like::bit::BitModelClassification::default(),
                "provider":{"provider_name":provider,"model_id":"inline-model","params":{}}}),
            ..Default::default()
        }
    }
    #[test]
    fn public_projector_source_preserves_identity_and_signed_sources_are_rejected() {
        let mut bit = inline_model("local");
        let safe = "https://example.test/immutable/revision/projector.gguf";
        bit.parameters["provider"]["params"]["projection"] = serde_json::json!({
            "download_link":safe,"file_name":"projector.gguf","size":700});
        let original = bit.projection_bit().expect("projector");
        let public = public_bit_metadata(&bit).unwrap();
        assert_eq!(
            public
                .pointer("/parameters/provider/params/projection/download_link")
                .and_then(|value| value.as_str()),
            Some(safe)
        );
        let restored: flow_like::bit::Bit = serde_json::from_value(public).unwrap();
        assert_eq!(restored.projection_bit().unwrap().id, original.id);
        for unsafe_source in [
            "https://example.test/projector?signature=private",
            "https://user:private@example.test/projector",
            "https://example.test/projector#fragment",
            "http://example.test/projector",
        ] {
            bit.parameters["provider"]["params"]["projection"]["download_link"] =
                unsafe_source.into();
            let error = public_bit_metadata(&bit).unwrap_err().to_string();
            assert!(error.contains("signed or unsafe"));
            assert!(!error.contains("private"));
        }
    }
    #[test]
    fn pinned_inline_mlx_manifest_survives_metadata_sanitization() {
        let mut bit = inline_model("mlx");
        bit.parameters["huggingface"] = serde_json::json!({"schema":1,"repo_id":"owner/model","revision":"a".repeat(40),"format":"mlx","files":[
            {"path":"config.json","size":100},{"path":"tokenizer.json","size":200},
            {"path":"tokenizer_config.json","size":300},{"path":"processor_config.json","size":400},
            {"path":"model.safetensors","size":4000}]});
        let original = bit.inline_mlx_asset_bits().unwrap();
        assert_eq!(original.len(), 5);
        let public: flow_like::bit::Bit =
            serde_json::from_value(public_bit_metadata(&bit).unwrap()).unwrap();
        let actual = public.inline_mlx_asset_bits().unwrap();
        assert_eq!(
            actual.iter().map(|bit| &bit.id).collect::<Vec<_>>(),
            original.iter().map(|bit| &bit.id).collect::<Vec<_>>()
        );
        bit.parameters["huggingface"]["revision"] = "main".into();
        assert!(
            public_bit_metadata(&bit)
                .unwrap_err()
                .to_string()
                .contains("pinned repository revision")
        );
    }
    #[test]
    fn signed_media_urls_are_removed_from_nested_arrays() {
        let mut metadata = serde_json::json!({"preview_media":[
            "https://media.test/public.webp", "https://media.test/a?signature=private",
            ["https://user:private@media.test/a", "https://media.test/a#private", 7],
            {"urls":["HTTPS://media.test/a?token=private", "https://media.test/kept"]}
        ]});
        public_metadata(&mut metadata);
        assert_eq!(
            metadata,
            serde_json::json!({"preview_media":[
                "https://media.test/public.webp", [7], {"urls":["https://media.test/kept"]}
            ]})
        );
        assert!(!metadata.to_string().contains("private"));
    }
    #[test]
    fn model_credentials_are_not_exported_as_public_metadata() {
        assert!(
            reject_embedded_credentials(&serde_json::json!({"provider": {"api_key":"secret"}}))
                .is_err()
        );
        assert!(
            reject_embedded_credentials(&serde_json::json!({"api_key":null,"model":"model"}))
                .is_ok()
        );
        assert!(identifier("../escape").is_err());
    }
}
