use super::super::artifact_cache::{ARTIFACT_TTL, revision_key};
use crate::{
    cache::{CacheBackendHandle, best_effort},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::{RolePermissions, has_role_permission},
    routes::app::ensure_app_publicly_visible,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like::{
    flow::board::Board, state::FlowLikeState, utils::compression::from_compressed_with_meta,
};
use flow_like_storage::{
    Path as StoragePath,
    object_store::{ObjectStore, ObjectStoreExt},
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, sync::Arc};
use utoipa::ToSchema;

/// How many distinct node types to report. Enough to characterise what a
/// template does without shipping its graph.
const MAX_NODE_TYPES: usize = 12;
const CACHE_NAMESPACE: &str = "template-previews-v1";

/// A structural summary of a template — never the template itself.
///
/// This is the only template detail a non-member of the owning app can read. It
/// deliberately carries shape, not content: counts and node type names, no pin
/// values, no variable defaults, no comments, no connections. That is enough for
/// a caller to judge "is this a useful foundation?" and decide whether to fork
/// or acquire the app, without handing out the author's work.
#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct TemplatePreview {
    pub app_id: String,
    pub template_id: String,
    pub node_count: usize,
    pub layer_count: usize,
    pub variable_count: usize,
    /// Distinct node type names, capped at `MAX_NODE_TYPES` and sorted
    pub node_types: Vec<String>,
    /// True when `node_types` was truncated
    pub node_types_truncated: bool,
    /// Whether the template declares its own entry event node
    pub has_entry_event: bool,
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/templates/{template_id}/preview",
    tag = "templates",
    description = "Get a structural summary of a template: node/layer/variable counts and the node types it uses. Readable for any publicly visible app, so a template can be evaluated before forking or joining. Returns shape only, never the template contents.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("template_id" = String, Path, description = "Template ID")
    ),
    responses(
        (status = 200, description = "Structural summary of the template", body = TemplatePreview),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "The app is neither publicly visible nor readable by the caller"),
        (status = 404, description = "Template not found")
    )
)]
#[tracing::instrument(
    name = "GET /apps/{app_id}/templates/{template_id}/preview",
    skip(state, user)
)]
pub async fn get_template_preview(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, template_id)): Path<(String, String)>,
) -> Result<Json<TemplatePreview>, ApiError> {
    if !state.platform_config.features.unauthorized_read {
        user.sub()?;
    }

    ensure_preview_readable(&user, &app_id, &state).await?;

    // Master credentials: the caller was authorized by the app's visibility, not
    // by membership, so there is no scoped credential to issue for them.
    let app_state = state
        .master_state(&state)
        .await
        .map_err(|_| ApiError::NOT_FOUND)?;
    let store = Board::meta_store(&app_state)
        .await
        .map_err(|_| ApiError::NOT_FOUND)?;
    let storage_scope = state.storage_identity.meta.cache_scope();
    let preview = cached_preview(
        &state.cache,
        store,
        app_state,
        &app_id,
        &template_id,
        state.registry.fingerprint(),
        storage_scope.as_deref(),
    )
    .await
    .map_err(|_| ApiError::NOT_FOUND)?;
    Ok(Json(preview))
}

async fn cached_preview(
    cache: &CacheBackendHandle,
    store: Arc<dyn ObjectStore>,
    app_state: Arc<FlowLikeState>,
    app_id: &str,
    template_id: &str,
    catalog_fingerprint: [u8; 32],
    storage_scope: Option<&str>,
) -> flow_like_types::Result<TemplatePreview> {
    let board_dir = StoragePath::from("apps").join(app_id.to_owned());
    let path = board_dir.clone().join(format!("{template_id}.template"));
    let scope = storage_scope.map(|storage_scope| {
        (
            storage_scope,
            app_id,
            template_id,
            catalog_fingerprint,
            flow_like::flow::board::format::supported_version(),
        )
    });
    // A template version may be overwritten by imports. Its storage identity,
    // rather than a DB timestamp or semantic version, decides cache reuse.
    if let Some(scope) = scope
        && let Ok(meta) = store.head(&path).await
        && let Some(key) = revision_key(scope, &meta)
        && let Some(preview) = best_effort::get(cache, CACHE_NAMESPACE, &key).await
    {
        return Ok(preview);
    }

    let (proto, meta) = from_compressed_with_meta(store, path).await?;
    let template = Board::from_loaded_template_proto(proto, board_dir, app_state).await?;
    let preview = summarize_template(app_id, template_id, &template);
    if let Some(scope) = scope
        && let Some(key) = revision_key(scope, &meta)
    {
        best_effort::set(cache, CACHE_NAMESPACE, &key, &preview, ARTIFACT_TTL).await;
    }
    Ok(preview)
}

/// A preview is readable when the app is publicly visible, or when the caller is
/// a member who may read templates. Members are covered explicitly so the
/// endpoint keeps working for `Private` and `Prototype` apps, where the
/// visibility check alone would refuse the app's own team.
async fn ensure_preview_readable(
    user: &AppUser,
    app_id: &str,
    state: &AppState,
) -> Result<(), ApiError> {
    if ensure_app_publicly_visible(app_id, state).await.is_ok() {
        return Ok(());
    }

    let permission = user.app_permission(app_id, state).await?;
    if has_role_permission(&permission.permissions, RolePermissions::ReadTemplates) {
        return Ok(());
    }

    Err(ApiError::FORBIDDEN)
}

fn summarize_template(app_id: &str, template_id: &str, template: &Board) -> TemplatePreview {
    let mut node_types: BTreeSet<String> = BTreeSet::new();
    let mut has_entry_event = false;

    for node in template.nodes.values() {
        node_types.insert(node.name.clone());
        if node.start.unwrap_or(false) {
            has_entry_event = true;
        }
    }

    let node_types_truncated = node_types.len() > MAX_NODE_TYPES;

    TemplatePreview {
        app_id: app_id.to_string(),
        template_id: template_id.to_string(),
        node_count: template.nodes.len(),
        layer_count: template.layers.len(),
        variable_count: template.variables.len(),
        node_types: node_types.into_iter().take(MAX_NODE_TYPES).collect(),
        node_types_truncated,
        has_entry_event,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::app::artifact_cache::test_support::TestStore;
    use flow_like::{
        flow::{
            pin::ValueType,
            variable::{Variable, VariableType},
        },
        state::FlowLikeConfig,
        utils::{compression::compress_to_file, http::HTTPClient},
    };
    use flow_like_storage::{files::store::FlowLikeStore, object_store::PutPayload};
    use flow_like_types::ToProto;
    use std::sync::atomic::Ordering;

    fn app_state(store: Arc<TestStore>) -> Arc<FlowLikeState> {
        Arc::new(FlowLikeState::new(
            FlowLikeConfig::with_default_store(FlowLikeStore::Other(store)),
            HTTPClient::new_without_refetch(),
        ))
    }

    fn template() -> Board {
        Board::new_detached(Some("template".into()), StoragePath::from("apps/app"))
    }

    fn add_variable(template: &mut Board) {
        let variable = Variable::new("input", VariableType::String, ValueType::Normal);
        template.variables.insert(variable.id.clone(), variable);
    }

    #[flow_like_types::tokio::test]
    async fn previews_reuse_across_replicas_and_track_storage_catalog_and_bucket() {
        let first = CacheBackendHandle::memory_for_test();
        let second = CacheBackendHandle::from_store_for_test(first.store().await.unwrap());
        let store = Arc::new(TestStore::default());
        let state = app_state(store.clone());
        let path = StoragePath::from("apps/app/template.template");
        let mut template = template();
        compress_to_file(store.clone(), path.clone(), &template.to_proto())
            .await
            .unwrap();

        for cache in [&first, &second] {
            assert_eq!(
                cached_preview(
                    cache,
                    store.clone(),
                    state.clone(),
                    "app",
                    "template",
                    [0; 32],
                    Some("bucket")
                )
                .await
                .unwrap()
                .variable_count,
                0
            );
        }
        assert_eq!(store.reads.load(Ordering::SeqCst), 1);
        cached_preview(
            &second,
            store.clone(),
            state.clone(),
            "app",
            "template",
            [1; 32],
            Some("bucket"),
        )
        .await
        .unwrap();
        assert_eq!(
            store.reads.load(Ordering::SeqCst),
            2,
            "node schema changes must rehydrate"
        );
        cached_preview(
            &second,
            store.clone(),
            state.clone(),
            "app",
            "template",
            [0; 32],
            Some("other-bucket"),
        )
        .await
        .unwrap();
        assert_eq!(store.reads.load(Ordering::SeqCst), 3);

        // Keep the semantic version and timestamps unchanged, as an import can.
        add_variable(&mut template);
        compress_to_file(store.clone(), path.clone(), &template.to_proto())
            .await
            .unwrap();
        assert_eq!(
            cached_preview(
                &second,
                store.clone(),
                state.clone(),
                "app",
                "template",
                [0; 32],
                Some("bucket")
            )
            .await
            .unwrap()
            .variable_count,
            1
        );
        assert_eq!(store.reads.load(Ordering::SeqCst), 4);

        store.delete(&path).await.unwrap();
        assert!(
            cached_preview(
                &second,
                store.clone(),
                state.clone(),
                "app",
                "template",
                [0; 32],
                Some("bucket")
            )
            .await
            .is_err()
        );
        store
            .put(&path, PutPayload::from_static(b"invalid template"))
            .await
            .unwrap();
        assert!(
            cached_preview(
                &second,
                store,
                state,
                "app",
                "template",
                [0; 32],
                Some("bucket")
            )
            .await
            .is_err()
        );
    }

    #[flow_like_types::tokio::test]
    async fn racing_template_rewrite_uses_the_get_identity() {
        let cache = CacheBackendHandle::memory_for_test();
        let store = Arc::new(TestStore::default());
        let state = app_state(store.clone());
        let path = StoragePath::from("apps/app/template.template");
        let mut template = template();
        compress_to_file(store.clone(), path.clone(), &template.to_proto())
            .await
            .unwrap();
        let old_meta = store.head(&path).await.unwrap();
        add_variable(&mut template);
        let replacement = StoragePath::from("replacement");
        compress_to_file(store.clone(), replacement.clone(), &template.to_proto())
            .await
            .unwrap();
        let bytes = store
            .inner
            .get(&replacement)
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        *store.after_head.lock().unwrap() = Some((path, PutPayload::from(bytes)));

        assert_eq!(
            cached_preview(
                &cache,
                store.clone(),
                state.clone(),
                "app",
                "template",
                [0; 32],
                Some("bucket")
            )
            .await
            .unwrap()
            .variable_count,
            1
        );
        let old_key = revision_key(
            (
                "bucket",
                "app",
                "template",
                [0_u8; 32],
                flow_like::flow::board::format::supported_version(),
            ),
            &old_meta,
        )
        .unwrap();
        assert!(
            cache
                .platform()
                .await
                .unwrap()
                .get::<TemplatePreview>(CACHE_NAMESPACE, &old_key)
                .await
                .unwrap()
                .is_none()
        );
        let reads = store.reads.load(Ordering::SeqCst);
        cached_preview(
            &cache,
            store.clone(),
            state,
            "app",
            "template",
            [0; 32],
            Some("bucket"),
        )
        .await
        .unwrap();
        assert_eq!(store.reads.load(Ordering::SeqCst), reads);
    }

    #[flow_like_types::tokio::test]
    async fn cached_preview_does_not_bypass_request_format_support() {
        use flow_like::flow::board::format::{
            CURRENT_BOARD_FORMAT_VERSION, with_supported_version,
        };

        let cache = CacheBackendHandle::memory_for_test();
        let store = Arc::new(TestStore::default());
        let state = app_state(store.clone());
        let mut template = template();
        template.format_version = CURRENT_BOARD_FORMAT_VERSION;
        compress_to_file(
            store.clone(),
            StoragePath::from("apps/app/template.template"),
            &template.to_proto(),
        )
        .await
        .unwrap();
        assert!(
            cached_preview(
                &cache,
                store.clone(),
                state.clone(),
                "app",
                "template",
                [0; 32],
                Some("bucket")
            )
            .await
            .is_ok()
        );
        let result = with_supported_version(
            CURRENT_BOARD_FORMAT_VERSION - 1,
            cached_preview(
                &cache,
                store.clone(),
                state,
                "app",
                "template",
                [0; 32],
                Some("bucket"),
            ),
        )
        .await;
        assert!(result.is_err());
        assert_eq!(store.reads.load(Ordering::SeqCst), 2);
    }
}
