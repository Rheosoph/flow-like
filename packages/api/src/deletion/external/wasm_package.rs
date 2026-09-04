use flow_like_storage::object_store::path::Path;

use super::delete_prefix;
use crate::error::ApiError;
use crate::routes::registry::server::{
    WASM_COMPILED_PATH, WIDGET_ASSETS_PATH, WIDGET_BUNDLES_PATH,
};
use crate::state::AppState;

/// Mirrors the private `WASM_PACKAGES_PATH` of the registry server.
const WASM_PACKAGES_PATH: &str = "wasm";

/// Every stored artifact of the package across all versions: the `.wasm`
/// binaries, widget bundles and unpacked widget assets on the content store,
/// and the compiled `.cwasm` binaries on the meta store. Prefix-based, so it
/// does not depend on the version rows that drain afterwards.
pub async fn delete_artifacts(state: &AppState, package_id: &str) -> Result<(), ApiError> {
    let credentials = state.master_credentials().await?;
    let content = credentials.to_store(false).await?.as_generic();
    let meta = credentials.to_store(true).await?.as_generic();
    for (label, prefix) in [
        ("wasm", Path::from(WASM_PACKAGES_PATH).child(package_id)),
        (
            "widget bundles",
            Path::from(WIDGET_BUNDLES_PATH).child(package_id),
        ),
        (
            "widget assets",
            Path::from(WIDGET_ASSETS_PATH).child(package_id),
        ),
    ] {
        delete_prefix(&content, &prefix, label).await?;
    }
    delete_prefix(
        &meta,
        &Path::from(WASM_COMPILED_PATH).child(package_id),
        "compiled wasm",
    )
    .await?;
    Ok(())
}
