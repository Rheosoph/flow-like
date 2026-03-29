use std::sync::Arc;

use flow_like_storage::{Path, files::store::FlowLikeStore};
use flow_like_types::{Result, anyhow};

use crate::state::FlowLikeState;

pub async fn construct_storage(
    state: &Arc<FlowLikeState>,
    app_id: &str,
    prefix: &str,
) -> Result<(FlowLikeStore, Path)> {
    let project_store = state
        .config
        .read()
        .await
        .stores
        .app_storage_store
        .clone()
        .ok_or(anyhow!("Project store not found"))?;

    let base_path = project_store.construct_upload(app_id, prefix).await?;

    Ok((project_store, base_path))
}

pub async fn construct_user_storage(
    state: &Arc<FlowLikeState>,
    sub: &str,
    app_id: &str,
    prefix: &str,
) -> Result<(FlowLikeStore, Path)> {
    let project_store = state
        .config
        .read()
        .await
        .stores
        .app_storage_store
        .clone()
        .ok_or(anyhow!("Project store not found"))?;

    let base_path = project_store
        .construct_user_upload(sub, app_id, prefix)
        .await?;

    Ok((project_store, base_path))
}
