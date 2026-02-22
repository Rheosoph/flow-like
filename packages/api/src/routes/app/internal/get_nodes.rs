use crate::{
    ensure_permission, entity::{app_package, wasm_package_version}, error::ApiError, middleware::jwt::AppUser, permission::role_permission::RolePermissions, state::AppState
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like::flow::node::{Node, NodeWasm};
use flow_like_wasm::manifest::PackageNodeEntry;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

fn package_node_to_node(entry: &PackageNodeEntry, package_id: &str) -> Node {
    Node {
        id: entry.id.clone(),
        name: entry.name.clone(),
        friendly_name: entry
            .friendly_name
            .clone()
            .unwrap_or_else(|| entry.name.clone()),
        description: entry.description.clone(),
        coordinates: None,
        category: entry.category.clone(),
        scores: entry.scores.clone(),
        pins: entry.pins.clone(),
        start: entry.start,
        icon: entry.icon.clone(),
        comment: None,
        long_running: entry.long_running,
        error: None,
        docs: entry.docs.clone(),
        event_callback: entry.event_callback,
        layer: None,
        hash: None,
        fn_refs: entry.fn_refs.clone(),
        oauth_providers: if entry.oauth_providers.is_empty() {
            None
        } else {
            Some(entry.oauth_providers.clone())
        },
        required_oauth_scopes: entry.required_oauth_scopes.clone(),
        only_offline: entry.only_offline,
        version: entry.version,
        wasm: Some(NodeWasm {
            package_id: package_id.to_string(),
            permissions: entry.permissions.clone(),
        }),
    }
}

#[utoipa::path(
    get,
    path = "/apps/nodes",
    tag = "apps",
    responses(
        (status = 200, description = "List of available nodes", body = Vec<Object>),
        (status = 401, description = "Unauthorized")
    )
)]
#[tracing::instrument(name = "GET /apps/nodes", skip(state, user))]
pub async fn get_nodes(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
) -> Result<Json<Vec<Node>>, ApiError> {
    user.sub()?;

    let nodes = state.registry.as_ref();
    let nodes = nodes.get_nodes();

    Ok(Json(nodes))
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/nodes",
    tag = "apps",
    params(
        ("app_id" = String, Path, description = "Application ID")
    ),
    responses(
        (status = 200, description = "List of available nodes for this app", body = Vec<Object>),
        (status = 401, description = "Unauthorized")
    )
)]
#[tracing::instrument(name = "GET /apps/{app_id}/nodes", skip(state, user))]
pub async fn get_app_nodes(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
) -> Result<Json<Vec<Node>>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::ReadBoards);

    let packages = app_package::Entity::find()
        .filter(app_package::Column::AppId.eq(&app_id))
        .filter(app_package::Column::Stale.eq(false))
        .all(&state.db)
        .await?;

    let mut wasm_nodes: Vec<Node> = Vec::with_capacity(packages.len() * 5);

    for pkg in &packages {

        // The catalog does not need to show stale packages
        if pkg.stale {
            continue;
        }

        let version_record = wasm_package_version::Entity::find()
            .filter(wasm_package_version::Column::PackageId.eq(&pkg.package_id))
            .filter(wasm_package_version::Column::Version.eq(&pkg.version))
            .one(&state.db)
            .await?;

        let version_record = match version_record {
            Some(v) => v,
            None => continue,
        };

        let entries: Vec<PackageNodeEntry> =
            serde_json::from_value(version_record.nodes).unwrap_or_default();

        for entry in &entries {
            wasm_nodes.push(package_node_to_node(entry, &pkg.package_id));
        }
    }

    let mut nodes = state.registry.as_ref().get_nodes();
    nodes.extend(wasm_nodes);

    Ok(Json(nodes))
}
