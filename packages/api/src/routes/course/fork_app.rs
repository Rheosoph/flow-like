use crate::{
    credentials::CredentialsAccess,
    entity::{
        app, app_package, event, membership, meta, page, role,
        sea_orm_active_enums::{Status, Visibility},
    },
    error::ApiError,
    permission::role_permission::RolePermissions,
    state::AppState,
};
use flow_like::utils::compression::{compress_to_file, from_compressed};
use flow_like_storage::Path;
use flow_like_types::{anyhow, create_id, proto};
use futures_util::TryStreamExt;
use sea_orm::{
    ActiveModelTrait,
    ActiveValue::{NotSet, Set},
    ColumnTrait, DbErr, EntityTrait, IntoActiveModel, QueryFilter, TransactionTrait,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use utoipa::ToSchema;

/// Mapping table built during a fork. Returned to callers and persisted on
/// the user's enrollment so the server can later translate original-app IDs
/// (referenced by lesson payloads, app refs, etc.) into the user-specific
/// IDs in their forked copy.
#[derive(Clone, Debug, Default, Serialize, Deserialize, ToSchema)]
pub struct ForkIdMap {
    pub source_app_id: String,
    pub app_id: String,
    /// Board IDs: source -> destination
    pub boards: HashMap<String, String>,
    /// Node IDs: source -> destination (flat across boards & layers)
    pub nodes: HashMap<String, String>,
    /// Pin IDs: source -> destination (flat across boards, layers, nodes)
    pub pins: HashMap<String, String>,
    /// Event IDs: source -> destination
    pub events: HashMap<String, String>,
    /// Page IDs: source -> destination
    pub pages: HashMap<String, String>,
    /// Layer IDs: source -> destination
    pub layers: HashMap<String, String>,
}

impl ForkIdMap {
    pub fn translate_board(&self, src: &str) -> String {
        self.boards
            .get(src)
            .cloned()
            .unwrap_or_else(|| src.to_string())
    }
    pub fn translate_node(&self, src: &str) -> String {
        self.nodes
            .get(src)
            .cloned()
            .unwrap_or_else(|| src.to_string())
    }
    pub fn translate_event(&self, src: &str) -> String {
        self.events
            .get(src)
            .cloned()
            .unwrap_or_else(|| src.to_string())
    }
    pub fn translate_page(&self, src: &str) -> String {
        self.pages
            .get(src)
            .cloned()
            .unwrap_or_else(|| src.to_string())
    }
}

/// Forks a source app into a new user-owned copy with **all internal IDs
/// remapped** (boards, layers, nodes, pins, events, pages). Returns the
/// mapping table so the caller can persist it on the user's enrollment.
pub async fn fork_app(
    state: &AppState,
    user_sub: &str,
    src_app_id: &str,
    language: &str,
) -> Result<(String, ForkIdMap), ApiError> {
    let new_app_id = create_id();
    let now = chrono::Utc::now().naive_utc();

    let src_credentials = state.master_credentials().await?;
    let src_store = src_credentials.to_store(true).await?.as_generic();

    let dst_credentials = state
        .scoped_credentials(user_sub, &new_app_id, CredentialsAccess::EditApp)
        .await?;
    let dst_store = dst_credentials.to_store(true).await?.as_generic();

    let src_prefix = Path::from("apps").child(src_app_id.to_string());
    let dst_prefix = Path::from("apps").child(new_app_id.clone());

    // ---- 1. Load the source manifest ----------------------------------
    let manifest_path = src_prefix.child("manifest.app");
    let mut src_app_proto: proto::App = from_compressed(src_store.clone(), manifest_path.clone())
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("read source manifest: {e}")))?;

    // ---- 2. Pre-allocate top-level ID mappings ------------------------
    let mut maps = ForkIdMap {
        source_app_id: src_app_id.to_string(),
        app_id: new_app_id.clone(),
        ..Default::default()
    };
    for b in &src_app_proto.boards {
        maps.boards.insert(b.clone(), create_id());
    }
    for e in &src_app_proto.events {
        maps.events.insert(e.clone(), create_id());
    }
    for p in &src_app_proto.page_ids {
        maps.pages.insert(p.clone(), create_id());
    }

    // ---- 3. Load + remap each board, then save under new prefix -------
    let mut new_board_protos: Vec<(String, proto::Board)> = Vec::new();
    for src_board_id in &src_app_proto.boards {
        let board_path = src_prefix.child(format!("{}.board", src_board_id));
        let board_proto: proto::Board =
            match from_compressed::<proto::Board>(src_store.clone(), board_path).await {
                Ok(b) => b,
                Err(err) => {
                    tracing::warn!("skipping board {} during fork: {}", src_board_id, err);
                    continue;
                }
            };
        let remapped = remap_board(board_proto, &mut maps);
        new_board_protos.push((src_board_id.clone(), remapped));
    }

    for (src_board_id, board) in &new_board_protos {
        let new_board_id = maps.translate_board(src_board_id);
        let board_path = dst_prefix.child(format!("{}.board", new_board_id));
        compress_to_file(dst_store.clone(), board_path, board)
            .await
            .map_err(|e| ApiError::internal_error(anyhow!("write board: {e}")))?;

        // Pages live under apps/{app}/_{board_id}/{page_id}.page — copy and
        // remap each one.
        let src_pages_dir = src_prefix.child(format!("_{}", src_board_id));
        let dst_pages_dir = dst_prefix.child(format!("_{}", new_board_id));
        copy_and_remap_pages(
            &src_store,
            &dst_store,
            &src_pages_dir,
            &dst_pages_dir,
            &maps,
        )
        .await?;
    }

    // ---- 4. Remap each event file -------------------------------------
    let src_events_dir = src_prefix.child("events");
    let dst_events_dir = dst_prefix.child("events");
    let mut events_listing = src_store.list(Some(&src_events_dir));
    while let Some(item) = events_listing
        .try_next()
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("list events dir: {e}")))?
    {
        let path_str = item.location.as_ref().to_string();
        let suffix = match path_str.strip_prefix(src_events_dir.as_ref()) {
            Some(s) => s.trim_start_matches('/'),
            None => continue,
        };
        // Only handle current versions (top-level events/{event_id}.event).
        if suffix.contains('/') {
            continue;
        }
        let Some(file_name) = suffix.strip_suffix(".event") else {
            continue;
        };
        let src_event_id = file_name.to_string();
        let new_event_id = maps
            .events
            .get(&src_event_id)
            .cloned()
            .unwrap_or_else(create_id);
        maps.events
            .entry(src_event_id.clone())
            .or_insert_with(|| new_event_id.clone());

        let src_event_path = src_events_dir.child(format!("{}.event", src_event_id));
        let mut event_proto: proto::Event =
            match from_compressed(src_store.clone(), src_event_path).await {
                Ok(e) => e,
                Err(err) => {
                    tracing::warn!("skip event {}: {}", src_event_id, err);
                    continue;
                }
            };
        remap_event(&mut event_proto, &maps);
        let dst_event_path = dst_events_dir.child(format!("{}.event", new_event_id));
        compress_to_file(dst_store.clone(), dst_event_path, &event_proto)
            .await
            .map_err(|e| ApiError::internal_error(anyhow!("write event: {e}")))?;
    }

    // ---- 5. Copy metadata files unchanged -----------------------------
    let src_meta_dir = src_prefix.child("metadata");
    let dst_meta_dir = dst_prefix.child("metadata");
    copy_object_prefix(
        &src_store,
        &dst_store,
        &src_meta_dir,
        &dst_meta_dir,
        "metadata",
    )
    .await?;

    // ---- 6. Copy project-owned storage unchanged ----------------------
    // This intentionally excludes user-private storage under
    // users/{sub}/apps/{app}; course forks should only clone app-owned seed
    // files and app databases.
    copy_object_prefix(
        &src_store,
        &dst_store,
        &src_prefix.child("upload"),
        &dst_prefix.child("upload"),
        "upload storage",
    )
    .await?;
    copy_object_prefix(
        &src_store,
        &dst_store,
        &src_prefix.child("storage"),
        &dst_prefix.child("storage"),
        "app storage",
    )
    .await?;

    // ---- 7. Rewrite the manifest --------------------------------------
    src_app_proto.id = new_app_id.clone();
    src_app_proto.boards = src_app_proto
        .boards
        .iter()
        .map(|b| maps.translate_board(b))
        .collect();
    src_app_proto.events = src_app_proto
        .events
        .iter()
        .map(|e| maps.translate_event(e))
        .collect();
    src_app_proto.page_ids = src_app_proto
        .page_ids
        .iter()
        .map(|p| maps.translate_page(p))
        .collect();
    let mut new_routes = HashMap::new();
    for (path, event_id) in src_app_proto.route_mappings.iter() {
        new_routes.insert(path.clone(), maps.translate_event(event_id));
    }
    src_app_proto.route_mappings = new_routes;
    src_app_proto.visibility = proto::AppVisibility::Private as i32;
    src_app_proto.status = proto::AppStatus::Active as i32;

    compress_to_file(
        dst_store.clone(),
        dst_prefix.child("manifest.app"),
        &src_app_proto,
    )
    .await
    .map_err(|e| ApiError::internal_error(anyhow!("write manifest: {e}")))?;

    // ---- 8. DB: app row, meta, roles, membership, events, pages -------
    let src_app_row = app::Entity::find_by_id(src_app_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    let src_meta_rows = meta::Entity::find()
        .filter(meta::Column::AppId.eq(src_app_id))
        .all(&state.db)
        .await?;

    let src_event_rows = event::Entity::find()
        .filter(event::Column::AppId.eq(src_app_id))
        .all(&state.db)
        .await?;

    let src_page_rows = page::Entity::find()
        .filter(page::Column::AppId.eq(src_app_id))
        .all(&state.db)
        .await?;

    let src_package_rows = app_package::Entity::find()
        .filter(app_package::Column::AppId.eq(src_app_id))
        .all(&state.db)
        .await?;

    // Ensure every DB id has a mapping (in case storage and DB drift).
    for e in &src_event_rows {
        maps.events.entry(e.id.clone()).or_insert_with(create_id);
    }
    for p in &src_page_rows {
        maps.pages.entry(p.id.clone()).or_insert_with(create_id);
    }

    let new_app_id_db = new_app_id.clone();
    let user_sub_owned = user_sub.to_string();
    let language_owned = language.to_string();
    let maps_arc = Arc::new(maps.clone());

    state
        .db
        .transaction::<_, (), DbErr>(|txn| {
            Box::pin(async move {
                let new_app_model = app::ActiveModel {
                    id: Set(new_app_id_db.clone()),
                    status: Set(Status::Active),
                    visibility: Set(Visibility::Private),
                    changelog: Set(src_app_row.changelog.clone()),
                    default_role_id: NotSet,
                    owner_role_id: NotSet,
                    primary_category: Set(src_app_row.primary_category.clone()),
                    secondary_category: Set(src_app_row.secondary_category.clone()),
                    rating_sum: Set(0),
                    rating_count: Set(0),
                    download_count: Set(0),
                    interactions_count: Set(0),
                    avg_rating: Set(None),
                    relevance_score: Set(None),
                    total_size: Set(0),
                    price: Set(0),
                    version: Set(src_app_row.version.clone()),
                    execution_mode: Set(src_app_row.execution_mode.clone()),
                    bits: Set(src_app_row.bits.clone()),
                    created_at: Set(now),
                    updated_at: Set(now),
                };
                let inserted_app = new_app_model.insert(txn).await?;

                let mut have_lang = false;
                for m in &src_meta_rows {
                    if m.lang == language_owned {
                        have_lang = true;
                    }
                    let new_meta = meta::ActiveModel {
                        id: Set(create_id()),
                        lang: Set(m.lang.clone()),
                        name: Set(m.name.clone()),
                        description: Set(m.description.clone()),
                        long_description: Set(m.long_description.clone()),
                        release_notes: Set(m.release_notes.clone()),
                        tags: Set(m.tags.clone()),
                        use_case: Set(m.use_case.clone()),
                        icon: Set(m.icon.clone()),
                        thumbnail: Set(m.thumbnail.clone()),
                        preview_media: Set(m.preview_media.clone()),
                        age_rating: Set(m.age_rating),
                        website: Set(m.website.clone()),
                        support_url: Set(m.support_url.clone()),
                        docs_url: Set(m.docs_url.clone()),
                        organization_specific_values: Set(m.organization_specific_values.clone()),
                        app_id: Set(Some(new_app_id_db.clone())),
                        bit_id: Set(None),
                        course_id: Set(None),
                        template_id: Set(None),
                        widget_id: Set(None),
                        wasm_package_id: Set(None),
                        created_at: Set(now),
                        updated_at: Set(now),
                    };
                    new_meta.insert(txn).await?;
                }
                if !have_lang && src_meta_rows.is_empty() {
                    let fallback = meta::ActiveModel {
                        id: Set(create_id()),
                        lang: Set(language_owned.clone()),
                        name: Set("My copy".to_string()),
                        app_id: Set(Some(new_app_id_db.clone())),
                        created_at: Set(now),
                        updated_at: Set(now),
                        ..Default::default()
                    };
                    fallback.insert(txn).await?;
                }

                let owner_role = role::ActiveModel {
                    id: Set(create_id()),
                    name: Set("Owner".to_string()),
                    description: Set(Some("Owner role".to_string())),
                    permissions: Set(RolePermissions::Owner.bits()),
                    app_id: Set(Some(new_app_id_db.clone())),
                    attributes: NotSet,
                    created_at: Set(now),
                    updated_at: Set(now),
                };
                let owner_role = owner_role.insert(txn).await?;

                let admin_role = role::ActiveModel {
                    id: Set(create_id()),
                    name: Set("Admin".to_string()),
                    description: Set(Some("Admin role".to_string())),
                    permissions: Set(RolePermissions::Admin.bits()),
                    app_id: Set(Some(new_app_id_db.clone())),
                    attributes: NotSet,
                    created_at: Set(now),
                    updated_at: Set(now),
                };
                admin_role.insert(txn).await?;

                let mut user_perms = RolePermissions::ReadTemplates;
                user_perms.insert(RolePermissions::ExecuteEvents);
                user_perms.insert(RolePermissions::ListEvents);
                let user_role = role::ActiveModel {
                    id: Set(create_id()),
                    name: Set("User".to_string()),
                    description: Set(Some("User role".to_string())),
                    permissions: Set(user_perms.bits()),
                    app_id: Set(Some(new_app_id_db.clone())),
                    attributes: NotSet,
                    created_at: Set(now),
                    updated_at: Set(now),
                };
                let user_role = user_role.insert(txn).await?;

                let mut app_active = inserted_app.into_active_model();
                app_active.owner_role_id = Set(Some(owner_role.id.clone()));
                app_active.default_role_id = Set(Some(user_role.id.clone()));
                app_active.update(txn).await?;

                let owner_membership_id = create_id();
                let mship = membership::ActiveModel {
                    id: Set(owner_membership_id.clone()),
                    user_id: Set(user_sub_owned.clone()),
                    app_id: Set(new_app_id_db.clone()),
                    role_id: Set(owner_role.id.clone()),
                    joined_via: NotSet,
                    created_at: Set(now),
                    updated_at: Set(now),
                };
                mship.insert(txn).await?;

                for pkg in &src_package_rows {
                    let new_package = app_package::ActiveModel {
                        id: Set(create_id()),
                        app_id: Set(new_app_id_db.clone()),
                        membership_id: Set(Some(owner_membership_id.clone())),
                        package_id: Set(pkg.package_id.clone()),
                        version: Set(pkg.version.clone()),
                        added_at: Set(now),
                        auto_update: Set(pkg.auto_update),
                        stale: Set(pkg.stale),
                    };
                    new_package.insert(txn).await?;
                }

                for e in &src_event_rows {
                    let new_event_id = maps_arc
                        .events
                        .get(&e.id)
                        .cloned()
                        .unwrap_or_else(create_id);
                    let new_board_id = e.board_id.as_ref().map(|b| maps_arc.translate_board(b));
                    let new_node_id = e.node_id.as_ref().map(|n| maps_arc.translate_node(n));
                    let new_page_id = e.page_id.as_ref().map(|p| maps_arc.translate_page(p));

                    let new_event = event::ActiveModel {
                        id: Set(new_event_id),
                        app_id: Set(new_app_id_db.clone()),
                        name: Set(e.name.clone()),
                        description: Set(e.description.clone()),
                        event_type: Set(e.event_type.clone()),
                        active: Set(e.active),
                        priority: Set(e.priority),
                        board_id: Set(new_board_id),
                        board_version: Set(e.board_version.clone()),
                        node_id: Set(new_node_id),
                        page_id: Set(new_page_id),
                        route: Set(e.route.clone()),
                        is_default: Set(e.is_default),
                        event_version: Set(e.event_version.clone()),
                        execution_mode: Set(e.execution_mode.clone()),
                        variables: Set(e.variables.clone()),
                        config: Set(e.config.clone()),
                        inputs: Set(e.inputs.clone()),
                        notes: Set(e.notes.clone()),
                        canary: Set(e.canary.clone()),
                        created_at: Set(now),
                        updated_at: Set(now),
                    };
                    new_event.insert(txn).await?;
                }

                for p in &src_page_rows {
                    let new_page_id = maps_arc.pages.get(&p.id).cloned().unwrap_or_else(create_id);
                    let new_board_id = p.board_id.as_ref().map(|b| maps_arc.translate_board(b));
                    let new_page = page::ActiveModel {
                        id: Set(new_page_id),
                        name: Set(p.name.clone()),
                        description: Set(p.description.clone()),
                        app_id: Set(new_app_id_db.clone()),
                        board_id: Set(new_board_id),
                        version: Set(p.version.clone()),
                        created_at: Set(now),
                        updated_at: Set(now),
                    };
                    new_page.insert(txn).await?;
                }

                Ok(())
            })
        })
        .await
        .map_err(|e| match e {
            sea_orm::TransactionError::Connection(err) => ApiError::from(err),
            sea_orm::TransactionError::Transaction(err) => ApiError::from(err),
        })?;

    Ok((new_app_id, maps))
}

// ---- helpers ----------------------------------------------------------

fn remap_board(mut board: proto::Board, maps: &mut ForkIdMap) -> proto::Board {
    let new_board_id = maps
        .boards
        .get(&board.id)
        .cloned()
        .unwrap_or_else(create_id);
    maps.boards
        .entry(board.id.clone())
        .or_insert_with(|| new_board_id.clone());

    // First pass: build node + pin id maps for this board.
    register_node_pin_ids(&board.nodes, maps);
    for layer in board.layers.values() {
        register_node_pin_ids(&layer.nodes, maps);
        register_pin_ids(&layer.pins, maps);
    }

    // Second pass: rewrite ids and references.
    let mut new_nodes = HashMap::with_capacity(board.nodes.len());
    for (_, mut node) in board.nodes.drain() {
        rewrite_node(&mut node, maps);
        new_nodes.insert(node.id.clone(), node);
    }
    board.nodes = new_nodes;

    let mut new_layers = HashMap::with_capacity(board.layers.len());
    for (_, mut layer) in board.layers.drain() {
        let new_layer_id = maps
            .layers
            .entry(layer.id.clone())
            .or_insert_with(create_id)
            .clone();
        layer.id = new_layer_id.clone();
        if let Some(parent) = layer.parent_id.as_ref() {
            layer.parent_id = Some(
                maps.layers
                    .entry(parent.clone())
                    .or_insert_with(create_id)
                    .clone(),
            );
        }
        let mut layer_nodes = HashMap::with_capacity(layer.nodes.len());
        for (_, mut node) in layer.nodes.drain() {
            rewrite_node(&mut node, maps);
            layer_nodes.insert(node.id.clone(), node);
        }
        layer.nodes = layer_nodes;

        let mut layer_pins = HashMap::with_capacity(layer.pins.len());
        for (_, mut pin) in layer.pins.drain() {
            rewrite_pin_top(&mut pin, maps);
            layer_pins.insert(pin.id.clone(), pin);
        }
        layer.pins = layer_pins;
        new_layers.insert(new_layer_id, layer);
    }
    board.layers = new_layers;

    board.id = new_board_id;
    board.page_ids = board
        .page_ids
        .iter()
        .map(|p| maps.translate_page(p))
        .collect();
    board
}

fn register_node_pin_ids(nodes: &HashMap<String, proto::Node>, maps: &mut ForkIdMap) {
    for (_, node) in nodes.iter() {
        maps.nodes.entry(node.id.clone()).or_insert_with(create_id);
        register_pin_ids(&node.pins, maps);
    }
}

fn register_pin_ids(pins: &HashMap<String, proto::Pin>, maps: &mut ForkIdMap) {
    for (_, pin) in pins.iter() {
        maps.pins.entry(pin.id.clone()).or_insert_with(create_id);
    }
}

fn rewrite_node(node: &mut proto::Node, maps: &ForkIdMap) {
    node.id = maps.translate_node(&node.id);
    if let Some(layer) = node.layer.as_ref() {
        node.layer = maps.layers.get(layer).cloned();
    }
    let mut new_pins = HashMap::with_capacity(node.pins.len());
    for (_, mut pin) in node.pins.drain() {
        rewrite_pin_top(&mut pin, maps);
        new_pins.insert(pin.id.clone(), pin);
    }
    node.pins = new_pins;
}

fn rewrite_pin_top(pin: &mut proto::Pin, maps: &ForkIdMap) {
    pin.id = maps.pins.get(&pin.id).cloned().unwrap_or(pin.id.clone());
    pin.connected_to = pin
        .connected_to
        .iter()
        .map(|p| maps.pins.get(p).cloned().unwrap_or(p.clone()))
        .collect();
    pin.depends_on = pin
        .depends_on
        .iter()
        .map(|p| maps.pins.get(p).cloned().unwrap_or(p.clone()))
        .collect();
}

fn remap_event(event: &mut proto::Event, maps: &ForkIdMap) {
    event.id = maps
        .events
        .get(&event.id)
        .cloned()
        .unwrap_or_else(create_id);
    event.board_id = maps.translate_board(&event.board_id);
    event.node_id = maps.translate_node(&event.node_id);
    if let Some(default_page) = event.default_page_id.as_ref() {
        event.default_page_id = Some(maps.translate_page(default_page));
    }
    if let Some(canary) = event.canary.as_mut() {
        canary.board_id = maps.translate_board(&canary.board_id);
        canary.node_id = maps.translate_node(&canary.node_id);
    }
    for input in event.inputs.iter_mut() {
        if let Some(new_pin) = maps.pins.get(&input.id) {
            input.id = new_pin.clone();
        }
    }
}

async fn copy_object_prefix(
    src_store: &Arc<dyn flow_like_storage::object_store::ObjectStore>,
    dst_store: &Arc<dyn flow_like_storage::object_store::ObjectStore>,
    src_prefix: &Path,
    dst_prefix: &Path,
    label: &str,
) -> Result<(), ApiError> {
    let mut listing = src_store.list(Some(src_prefix));
    while let Some(item) = listing
        .try_next()
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("list {label}: {e}")))?
    {
        let path_str = item.location.as_ref().to_string();
        let suffix = match path_str.strip_prefix(src_prefix.as_ref()) {
            Some(s) if s.is_empty() || s.starts_with('/') => s.trim_start_matches('/'),
            Some(_) => continue,
            None => continue,
        };
        let dst_path = if suffix.is_empty() {
            dst_prefix.clone()
        } else {
            dst_prefix.child(suffix.to_string())
        };
        let bytes = src_store
            .get(&item.location)
            .await
            .map_err(|e| ApiError::internal_error(anyhow!("read {label}: {e}")))?
            .bytes()
            .await
            .map_err(|e| ApiError::internal_error(anyhow!("read {label} bytes: {e}")))?;
        dst_store
            .put(&dst_path, bytes.into())
            .await
            .map_err(|e| ApiError::internal_error(anyhow!("write {label}: {e}")))?;
    }
    Ok(())
}

async fn copy_and_remap_pages(
    src_store: &Arc<dyn flow_like_storage::object_store::ObjectStore>,
    dst_store: &Arc<dyn flow_like_storage::object_store::ObjectStore>,
    src_pages_dir: &Path,
    dst_pages_dir: &Path,
    maps: &ForkIdMap,
) -> Result<(), ApiError> {
    let mut listing = src_store.list(Some(src_pages_dir));
    while let Some(item) = listing
        .try_next()
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("list pages dir: {e}")))?
    {
        let path_str = item.location.as_ref().to_string();
        let suffix = match path_str.strip_prefix(src_pages_dir.as_ref()) {
            Some(s) => s.trim_start_matches('/'),
            None => continue,
        };
        let Some(file_name) = suffix.strip_suffix(".page") else {
            continue;
        };
        let src_page_id = file_name.to_string();
        let new_page_id = maps.translate_page(&src_page_id);

        let mut page_proto: proto::Page =
            match from_compressed(src_store.clone(), item.location.clone()).await {
                Ok(p) => p,
                Err(err) => {
                    tracing::warn!("skip page {}: {}", src_page_id, err);
                    continue;
                }
            };
        page_proto.id = new_page_id.clone();
        if let Some(b) = page_proto.board_id.as_ref() {
            page_proto.board_id = Some(maps.translate_board(b));
        }
        if let Some(n) = page_proto.on_load_event_id.as_ref() {
            page_proto.on_load_event_id = Some(maps.translate_event(n));
        }
        if let Some(n) = page_proto.on_unload_event_id.as_ref() {
            page_proto.on_unload_event_id = Some(maps.translate_event(n));
        }
        if let Some(n) = page_proto.on_interval_event_id.as_ref() {
            page_proto.on_interval_event_id = Some(maps.translate_event(n));
        }

        let dst_path = dst_pages_dir.child(format!("{}.page", new_page_id));
        compress_to_file(dst_store.clone(), dst_path, &page_proto)
            .await
            .map_err(|e| ApiError::internal_error(anyhow!("write page: {e}")))?;
    }
    Ok(())
}
