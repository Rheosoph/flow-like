use crate::{
    credentials::CredentialsAccess,
    entity::{
        app, app_package, event, event_sink, membership, meta, page, role,
        sea_orm_active_enums::{Status, Visibility, WasmPackageVisibility},
        template, wasm_package, wasm_package_author, wasm_package_purchase, wasm_package_user,
        widget,
    },
    error::ApiError,
    permission::role_permission::RolePermissions,
    state::AppState,
};

pub mod cleanup;
pub mod preview;
use flow_like::utils::compression::{
    compress_to_file, compress_to_file_json, from_compressed, from_compressed_json,
};
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

/// Where the caller wants the destination app to land. Drives the
/// transport (server-side copy vs. client bundle) and the permission gate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub enum ForkTarget {
    /// Destination is the same backend as the source — server can use
    /// native `CopyObject` (S3/GCS/Azure) or fall back to byte copy.
    OnlineSameStore,
    /// Destination is a different online backend (cross-org/cross-deployment)
    /// — falls back to streaming get→put across stores.
    OnlineCrossStore,
    /// Destination is a desktop/offline client — server returns a manifest
    /// and signed-read URL, client downloads + applies locally.
    OfflineBundle,
}

/// Reason an item was skipped during a fork. Carried in `ForkReport.skipped`
/// so callers can show the user *why* something didn't make it.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub enum SkippedKind {
    /// A WASM package the target user can't access and isn't public+free
    Package,
    /// A bit unavailable in the destination environment
    Bit,
    /// A remote-event token site the caller didn't supply a token for, or
    /// where the token couldn't be reused (e.g. OAuth needs re-auth)
    RemoteEvent,
    /// A file exceeded a per-file or per-fork cap
    LargeFile,
    /// A secret variable / pin / event field whose value was cleared
    Secret,
    /// OAuth tokens cleared on the destination — user must re-link providers
    OAuthRequiresReauth,
    /// Anything else (storage list errors, malformed payloads, etc.)
    Other,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct SkippedItem {
    pub kind: SkippedKind,
    pub source_id: String,
    /// Human-readable reason — surfaced verbatim in the fork UI
    pub reason: String,
}

/// Typed entry point for callers. Replaces the positional `fork_app(state,
/// user_sub, src_app_id, language)` signature. Internal callers (e.g. the
/// course flow) can set `bypass_allow_forking_check` to skip the project's
/// `allow_forking` opt-in; external API endpoints must leave it `false`.
#[derive(Debug, Clone)]
pub struct ForkOptions<'a> {
    pub source_app_id: &'a str,
    /// `None` for the unauthenticated public→offline path
    pub target_user_sub: Option<&'a str>,
    pub target_mode: ForkTarget,
    pub language: &'a str,
    /// Single token reused across all PAT/HTTP-auth/cron sites. OAuth sites
    /// are still cleared and reported in `ForkReport.skipped`.
    pub remote_event_token: Option<&'a str>,
    /// Override destination visibility. Default for online targets is
    /// `Private`; for `OfflineBundle` it's forced to `Offline`.
    pub requested_visibility: Option<flow_like::app::AppVisibility>,
    /// Copy versioned files referenced by events/pages/templates. Default `true`.
    pub include_versions_pointed_to: bool,
    /// Trusted-internal escape hatch (course flow). Skips the
    /// `app.allow_forking` opt-in check; permission checks still apply.
    pub bypass_allow_forking_check: bool,
}

impl<'a> ForkOptions<'a> {
    /// Default options matching the existing course-fork behavior:
    /// online same-store, no token, no anonymous caller, copy versions.
    pub fn for_user(source_app_id: &'a str, user_sub: &'a str, language: &'a str) -> Self {
        Self {
            source_app_id,
            target_user_sub: Some(user_sub),
            target_mode: ForkTarget::OnlineSameStore,
            language,
            remote_event_token: None,
            requested_visibility: None,
            include_versions_pointed_to: true,
            bypass_allow_forking_check: false,
        }
    }
}

/// Per-fork report. Returned to the caller so the UI can show what was
/// skipped, what needs re-auth, and the totals copied. Serializable so
/// online forks can return it as JSON and offline-bundle forks can embed
/// it in the bundle response.
#[derive(Clone, Debug, Default, Serialize, Deserialize, ToSchema)]
pub struct ForkReport {
    pub id_map: ForkIdMap,
    #[serde(default)]
    pub skipped: Vec<SkippedItem>,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub bytes_copied: u64,
    #[serde(default)]
    pub objects_copied: u64,
}

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
    /// Widget IDs: source -> destination
    #[serde(default)]
    pub widgets: HashMap<String, String>,
    /// Template IDs: source -> destination
    #[serde(default)]
    pub templates: HashMap<String, String>,
    /// Variable IDs: source -> destination (currently identity-mapped —
    /// pins do not directly reference variable IDs in the proto today,
    /// so the map is reserved for future use and reporting)
    #[serde(default)]
    pub variables: HashMap<String, String>,
    /// Role IDs: source -> destination. Includes both the system roles
    /// (Owner / Admin / Member) and any custom roles copied from source.
    #[serde(default)]
    pub roles: HashMap<String, String>,
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

/// One serialized meta artifact extracted from the in-memory bundle —
/// the bytes are exactly what would have been written to disk
/// (compressed proto for boards/events/templates/manifest, compressed
/// JSON for widgets/pages). `path` is relative to `apps/{new_app_id}/`
/// so the desktop can write it back at the same offset.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct MetaBlob {
    /// Relative path under `apps/{new_app_id}/`, slash-delimited.
    pub relative_path: String,
    /// Base64-encoded compressed payload — exactly what would land on
    /// disk if the destination were materialized server-side.
    pub data_b64: String,
}

/// Result of running the fork pipeline against an in-memory destination
/// meta store. Returned by `fork_compute_offline_bundle` so the caller
/// can ship the bytes in an HTTP response and let the desktop write
/// them to its own local meta store.
pub struct OfflineMetaBundle {
    pub new_app_id: String,
    pub id_map: ForkIdMap,
    pub skipped: Vec<SkippedItem>,
    pub blobs: Vec<MetaBlob>,
}

/// Build an offline-fork bundle without going through the
/// `fork_app_with_visibility` storage pipeline.
///
/// **No server-side fork happens.** We don't allocate a destination
/// app id on the server's storage, don't write any persistent or
/// in-memory destination prefix, don't insert any DB row, don't
/// touch the content store. The function:
///
/// 1. Reads source meta artifacts (manifest, boards, events,
///    widgets, templates, pages, pointed-to versioned boards) and
///    overlays each with its DB-row counterpart where the DB is
///    authoritative — endpoints like `change_visibility`,
///    `change_forking`, `upsert_event` write the DB *and* don't
///    always rewrite `manifest.app` / `*.event` on storage, so
///    pulling raw from storage ships drift to the desktop.
/// 2. Allocates a fresh destination app id + a `ForkIdMap` covering
///    all of the source's IDs.
/// 3. Runs `remap_*` / `strip_*_secrets` on each artifact in
///    memory.
/// 4. Drops events whose `execution_mode == "Remote"` — offline
///    apps run only Local events. `event_type` is irrelevant here:
///    api / cron / webhook events that the user marked Local stay in
///    the bundle and execute on the desktop.
/// 5. Base64-encodes each remapped blob with its `apps/{new_app_id}/`-
///    relative path.
///
/// User-content (`metadata/`, `upload/`, `storage/`) is **not** in
/// the bundle — `begin_offline_fork` hands the desktop a single
/// scoped `ReadAppContent` credential over the source's content
/// prefix and the desktop pulls those bytes directly.
pub async fn compute_offline_fork_bundle(
    state: &AppState,
    src_app_id: &str,
) -> Result<OfflineMetaBundle, ApiError> {
    use crate::routes::app::events::db::db_model_to_event;
    use base64::Engine as _;
    use flow_like_types::ToProto;

    let credentials = state.master_credentials().await?;
    let src_meta_store = credentials.to_store(true).await?.as_generic();

    let src_prefix = Path::from("apps").child(src_app_id.to_string());
    let new_app_id = create_id();

    // ---- 1. Load manifest from storage, overlay DB row -------------
    // The manifest.app file on disk reflects state from the last time
    // a full-app save ran. Endpoints like `change_visibility`,
    // `change_forking`, and `upsert_app` update only the App DB row
    // and never rewrite the file — so the file's `visibility`,
    // `price`, `version`, `execution_mode`, `allow_forking`, etc. can
    // be arbitrarily stale. Overlay the DB row's authoritative values
    // before remap so the bundle ships current state to the desktop.
    let mut manifest_proto: proto::App =
        from_compressed(src_meta_store.clone(), src_prefix.child("manifest.app"))
            .await
            .map_err(|e| ApiError::internal_error(anyhow!("read source manifest: {e}")))?;
    overlay_app_row_into_manifest(state, src_app_id, &mut manifest_proto).await?;

    // ---- ID allocation: union(manifest, DB) -----------------------
    // The manifest may be stale relative to the DB (events / pages /
    // widgets / templates each have their own DB rows that some
    // endpoints update without rewriting manifest.app). Allocate dst
    // IDs from BOTH sources so a row that exists only in the DB
    // still gets a fresh translated id and doesn't ship with the
    // source's id.
    let event_id_set: Vec<String> = event::Entity::find()
        .filter(event::Column::AppId.eq(src_app_id))
        .all(&state.db)
        .await?
        .into_iter()
        .map(|r| r.id)
        .collect();
    let page_rows: Vec<page::Model> = page::Entity::find()
        .filter(page::Column::AppId.eq(src_app_id))
        .all(&state.db)
        .await?;
    let page_id_set: Vec<String> = page_rows.iter().map(|r| r.id.clone()).collect();
    let widget_id_set: Vec<String> = widget::Entity::find()
        .filter(widget::Column::AppId.eq(src_app_id))
        .all(&state.db)
        .await?
        .into_iter()
        .map(|r| r.id)
        .collect();
    let template_id_set: Vec<String> = template::Entity::find()
        .filter(template::Column::AppId.eq(src_app_id))
        .all(&state.db)
        .await?
        .into_iter()
        .map(|r| r.id)
        .collect();

    let mut maps = ForkIdMap {
        source_app_id: src_app_id.to_string(),
        app_id: new_app_id.clone(),
        ..Default::default()
    };
    for b in &manifest_proto.boards {
        // Boards have no DB row — manifest is the only source of
        // truth.
        maps.boards.insert(b.clone(), create_id());
    }
    for e in manifest_proto.events.iter().chain(event_id_set.iter()) {
        maps.events.entry(e.clone()).or_insert_with(create_id);
    }
    for p in manifest_proto.page_ids.iter().chain(page_id_set.iter()) {
        maps.pages.entry(p.clone()).or_insert_with(create_id);
    }
    for w in manifest_proto.widget_ids.iter().chain(widget_id_set.iter()) {
        maps.widgets.entry(w.clone()).or_insert_with(create_id);
    }
    for t in manifest_proto
        .templates
        .iter()
        .chain(template_id_set.iter())
    {
        maps.templates.entry(t.clone()).or_insert_with(create_id);
    }

    let mut blobs: Vec<MetaBlob> = Vec::new();
    let mut skipped: Vec<SkippedItem> = Vec::new();
    // Source-id sets of resources we *actually* shipped a blob for.
    // The manifest's id lists are rebuilt from these sets so we never
    // ship a manifest reference that points at a missing blob — e.g.
    // if a manifest entry's DB row was deleted, or a storage read
    // failed. The desktop sees only IDs whose data is in the bundle.
    let mut shipped_events: std::collections::HashSet<String> = Default::default();
    let mut shipped_pages: std::collections::HashSet<String> = Default::default();
    let mut shipped_widgets: std::collections::HashSet<String> = Default::default();
    let mut shipped_templates: std::collections::HashSet<String> = Default::default();

    // ---- 2. Boards: load, remap (allocates internal IDs), strip ----
    for src_board_id in &manifest_proto.boards.clone() {
        let board_path = src_prefix.child(format!("{}.board", src_board_id));
        let board_proto: proto::Board =
            match from_compressed::<proto::Board>(src_meta_store.clone(), board_path).await {
                Ok(b) => b,
                Err(err) => {
                    tracing::warn!("skip board {} during offline bundle: {}", src_board_id, err);
                    continue;
                }
            };
        let dst_board_id = maps.translate_board(src_board_id);
        let remapped = remap_board(board_proto, &mut maps);
        // remap_board strips secrets + remaps internal IDs.
        let bytes = encode_proto(&remapped).await?;
        blobs.push(MetaBlob {
            relative_path: format!("{}.board", dst_board_id),
            data_b64: base64::engine::general_purpose::STANDARD.encode(bytes),
        });
    }

    // ---- 2b. Pages: DB-driven, board-scoped binary --------------
    // Pages live at the canonical `_{board_id}/{page_id}.page`
    // (compressed binary `proto::Page`). Source data may still be
    // sitting at the legacy app-level JSON path written by the
    // (now-removed) `App::save_page`, so the read tries both —
    // every page lands in the bundle at the canonical location only.
    for row in &page_rows {
        let src_page_id = row.id.clone();
        let new_page_id = maps.translate_page(&src_page_id);
        let new_board_id = row.board_id.as_ref().map(|b| maps.translate_board(b));

        let Some(dst_board_id) = new_board_id else {
            tracing::warn!(
                "skip page {} in offline bundle: row has no board_id",
                src_page_id
            );
            continue;
        };

        let mut page_proto = match read_source_page(
            &src_meta_store,
            &src_prefix,
            row.board_id.as_deref(),
            &src_page_id,
        )
        .await
        {
            Some(p) => p,
            None => {
                tracing::warn!(
                    "skip page {} in offline bundle: no readable source file",
                    src_page_id
                );
                continue;
            }
        };

        remap_page(&mut page_proto, &new_page_id, &maps);

        let bytes = encode_proto(&page_proto).await?;
        blobs.push(MetaBlob {
            relative_path: format!("_{}/{}.page", dst_board_id, new_page_id),
            data_b64: base64::engine::general_purpose::STANDARD.encode(bytes),
        });
        shipped_pages.insert(src_page_id);
    }

    // ---- 3. Events: DB-driven, drop Remote, remap, strip ----------
    // Read events from the DB rather than from `apps/{src}/events/`
    // on storage. The DB is authoritative — `upsert_event` writes
    // both, but downstream endpoints that flip a flag or change a
    // schedule write only the DB row, leaving the `.event` file
    // stale. Listing storage would ship that drift to the desktop.
    let mut pointed_board_versions: std::collections::HashSet<(String, (u32, u32, u32))> =
        std::collections::HashSet::new();
    let event_rows = event::Entity::find()
        .filter(event::Column::AppId.eq(src_app_id))
        .all(&state.db)
        .await?;
    for row in event_rows {
        let src_event_id = row.id.clone();
        let core_event = match db_model_to_event(row) {
            Ok(e) => e,
            Err(err) => {
                tracing::warn!("skip event {} (db→core conversion): {}", src_event_id, err);
                continue;
            }
        };
        let mut event_proto = core_event.to_proto();

        // **Filter Remote events.** Only events the user explicitly
        // marked `execution_mode = Remote` are dropped — those run
        // server-side and have no place in an offline bundle. Local
        // events of any trigger type (api, cron, webhook, …) stay.
        if is_remote_event(&event_proto) {
            skipped.push(SkippedItem {
                kind: SkippedKind::RemoteEvent,
                source_id: src_event_id.clone(),
                reason: format!(
                    "event {} (mode={:?}) is marked Remote and was dropped from the offline bundle",
                    src_event_id, event_proto.execution_mode,
                ),
            });
            continue;
        }

        // Note pointed-to board versions for step 4.
        if let Some(v) = event_proto.board_version.as_ref() {
            pointed_board_versions
                .insert((event_proto.board_id.clone(), (v.major, v.minor, v.patch)));
        }
        if let Some(canary) = event_proto.canary.as_ref()
            && let Some(v) = canary.board_version.as_ref()
        {
            pointed_board_versions.insert((canary.board_id.clone(), (v.major, v.minor, v.patch)));
        }

        let new_event_id = maps.translate_event(&src_event_id);
        remap_event(&mut event_proto, &maps);
        let bytes = encode_proto(&event_proto).await?;
        blobs.push(MetaBlob {
            relative_path: format!("events/{}.event", new_event_id),
            data_b64: base64::engine::general_purpose::STANDARD.encode(bytes),
        });
        shipped_events.insert(src_event_id);
    }

    // ---- 4. Versioned boards pointed to by surviving events --------
    for (src_board_id, version) in &pointed_board_versions {
        let dst_board_id = maps.translate_board(src_board_id);
        let src_versioned_path = src_prefix
            .child("versions")
            .child(src_board_id.clone())
            .child(format!("{}_{}_{}.board", version.0, version.1, version.2));
        let board_proto: proto::Board =
            match from_compressed::<proto::Board>(src_meta_store.clone(), src_versioned_path).await
            {
                Ok(b) => b,
                Err(err) => {
                    tracing::warn!(
                        "skip versioned board {} v{}.{}.{}: {}",
                        src_board_id,
                        version.0,
                        version.1,
                        version.2,
                        err
                    );
                    continue;
                }
            };
        let mut remapped = remap_board(board_proto, &mut maps);
        // remap_board allocates a fresh board.id; force it back to
        // the destination live-board id so the archive stays
        // addressable.
        remapped.id = dst_board_id.clone();
        let bytes = encode_proto(&remapped).await?;
        blobs.push(MetaBlob {
            relative_path: format!(
                "versions/{}/{}_{}_{}.board",
                dst_board_id, version.0, version.1, version.2
            ),
            data_b64: base64::engine::general_purpose::STANDARD.encode(bytes),
        });
    }

    // ---- 5. Widgets ------------------------------------------------
    for (src_widget_id, new_widget_id) in maps.widgets.clone().iter() {
        let src_path = src_prefix.child(format!("{}.widget", src_widget_id));
        let mut widget: flow_like_types::Value =
            match from_compressed_json(src_meta_store.clone(), src_path).await {
                Ok(w) => w,
                Err(err) => {
                    tracing::warn!("skip widget {}: {}", src_widget_id, err);
                    continue;
                }
            };
        if let Some(obj) = widget.as_object_mut() {
            obj.insert(
                "id".to_string(),
                flow_like_types::Value::String(new_widget_id.clone()),
            );
        }
        let bytes = encode_json(&widget).await?;
        blobs.push(MetaBlob {
            relative_path: format!("{}.widget", new_widget_id),
            data_b64: base64::engine::general_purpose::STANDARD.encode(bytes),
        });
        shipped_widgets.insert(src_widget_id.clone());
    }

    // ---- 6. Templates (proto::Board) ------------------------------
    let template_pairs: Vec<(String, String)> = maps
        .templates
        .iter()
        .map(|(s, d)| (s.clone(), d.clone()))
        .collect();
    for (src_template_id, new_template_id) in template_pairs {
        let src_path = src_prefix.child(format!("{}.template", src_template_id));
        let board_proto: proto::Board =
            match from_compressed::<proto::Board>(src_meta_store.clone(), src_path).await {
                Ok(b) => b,
                Err(err) => {
                    tracing::warn!("skip template {}: {}", src_template_id, err);
                    continue;
                }
            };
        let mut remapped = remap_board(board_proto, &mut maps);
        remapped.id = new_template_id.clone();
        let bytes = encode_proto(&remapped).await?;
        blobs.push(MetaBlob {
            relative_path: format!("{}.template", new_template_id),
            data_b64: base64::engine::general_purpose::STANDARD.encode(bytes),
        });
        shipped_templates.insert(src_template_id);
    }

    // ---- 7. Rewrite + ship the manifest ----------------------------
    // Rebuild each id list as the union of (manifest, DB rows) so a
    // resource that exists only in the DB still ends up on the
    // desktop's manifest. Source ids are deduped via HashSet, then
    // translated through the id map. Boards have no DB row, so they
    // stay manifest-driven.
    manifest_proto.id = new_app_id.clone();
    manifest_proto.boards = manifest_proto
        .boards
        .iter()
        .map(|b| maps.translate_board(b))
        .collect();

    // Manifest id lists = ids we *actually* shipped a blob for. This
    // closes the dangling-reference edge case: if a manifest entry's
    // DB row was deleted (or storage read failed), the loop didn't
    // produce a blob — so the manifest must not list it. Otherwise
    // the desktop would try to load a missing file and crash.
    manifest_proto.events = shipped_events
        .iter()
        .map(|e| maps.translate_event(e))
        .collect();
    manifest_proto.page_ids = shipped_pages
        .iter()
        .map(|p| maps.translate_page(p))
        .collect();
    manifest_proto.widget_ids = shipped_widgets
        .iter()
        .map(|w| translate_in_map(&maps.widgets, w))
        .collect();
    manifest_proto.templates = shipped_templates
        .iter()
        .map(|t| translate_in_map(&maps.templates, t))
        .collect();

    // Routes only land in the manifest if they point at an event
    // whose blob shipped — same dangling-reference guard.
    let mut new_routes = HashMap::new();
    for (path, event_id) in manifest_proto.route_mappings.iter() {
        if shipped_events.contains(event_id) {
            new_routes.insert(path.clone(), maps.translate_event(event_id));
        }
    }
    manifest_proto.route_mappings = new_routes;
    manifest_proto.visibility = proto::AppVisibility::Offline as i32;
    manifest_proto.status = proto::AppStatus::Active as i32;
    manifest_proto.forked_from = Some(src_app_id.to_string());
    manifest_proto.forked_at = Some(flow_like_types::Timestamp::from(
        std::time::SystemTime::now(),
    ));
    manifest_proto.allow_forking = Some(false);
    manifest_proto.rating_sum = 0;
    manifest_proto.rating_count = 0;
    manifest_proto.download_count = 0;
    manifest_proto.interaction_count = 0;
    manifest_proto.avg_rating = None;
    manifest_proto.relevance_score = None;

    let manifest_bytes = encode_proto(&manifest_proto).await?;
    blobs.push(MetaBlob {
        relative_path: "manifest.app".to_string(),
        data_b64: base64::engine::general_purpose::STANDARD.encode(manifest_bytes),
    });

    Ok(OfflineMetaBundle {
        new_app_id,
        id_map: maps,
        skipped,
        blobs,
    })
}

/// Overlay the App DB row's authoritative fields onto the manifest
/// proto loaded from `manifest.app`. Endpoints like
/// `change_visibility`, `change_forking`, and `internal::upsert_app`
/// update the DB row but never rewrite the manifest file, so the
/// file's fields can be arbitrarily stale. Pulling the DB row's
/// values forward gives the bundle current state without depending
/// on a recent full-app save.
async fn overlay_app_row_into_manifest(
    state: &AppState,
    src_app_id: &str,
    manifest: &mut proto::App,
) -> Result<(), ApiError> {
    let row = match app::Entity::find_by_id(src_app_id).one(&state.db).await? {
        Some(r) => r,
        None => return Ok(()), // No DB row at all → nothing to overlay.
    };

    manifest.status = match row.status {
        Status::Active => proto::AppStatus::Active as i32,
        Status::Inactive => proto::AppStatus::Inactive as i32,
        Status::Archived => proto::AppStatus::Archived as i32,
    };
    manifest.visibility = match row.visibility {
        Visibility::Public => proto::AppVisibility::Public as i32,
        Visibility::PublicRequestAccess => proto::AppVisibility::PublicRequestAccess as i32,
        Visibility::Private => proto::AppVisibility::Private as i32,
        Visibility::Prototype => proto::AppVisibility::Prototype as i32,
        Visibility::Offline => proto::AppVisibility::Offline as i32,
    };
    manifest.changelog = row.changelog.clone();
    manifest.price = Some(row.price as i32);
    manifest.version = row.version.clone();
    manifest.execution_mode = Some(match row.execution_mode {
        crate::entity::sea_orm_active_enums::ExecutionMode::Any => {
            proto::AppExecutionMode::Any as i32
        }
        crate::entity::sea_orm_active_enums::ExecutionMode::Local => {
            proto::AppExecutionMode::Local as i32
        }
        crate::entity::sea_orm_active_enums::ExecutionMode::Remote => {
            proto::AppExecutionMode::Remote as i32
        }
    });
    if let Some(bits) = row.bits {
        manifest.bits = bits;
    }
    manifest.allow_forking = Some(row.allow_forking);
    manifest.forked_from = row.forked_from.clone();
    manifest.forked_at = row.forked_at.map(|dt| {
        flow_like_types::Timestamp::from(
            std::time::UNIX_EPOCH + std::time::Duration::from_secs(dt.and_utc().timestamp() as u64),
        )
    });
    if let Some(cat) = row.primary_category.as_ref() {
        manifest.primary_category = Some(category_to_proto(cat));
    }
    if let Some(cat) = row.secondary_category.as_ref() {
        manifest.secondary_category = Some(category_to_proto(cat));
    }
    Ok(())
}

fn category_to_proto(c: &crate::entity::sea_orm_active_enums::Category) -> i32 {
    use crate::entity::sea_orm_active_enums::Category;
    match c {
        Category::Other => proto::AppCategory::Other as i32,
        Category::Productivity => proto::AppCategory::Productivity as i32,
        Category::Social => proto::AppCategory::Social as i32,
        Category::Entertainment => proto::AppCategory::Entertainment as i32,
        Category::Education => proto::AppCategory::Education as i32,
        Category::Health => proto::AppCategory::Health as i32,
        Category::Finance => proto::AppCategory::Finance as i32,
        Category::Lifestyle => proto::AppCategory::Lifestyle as i32,
        Category::Travel => proto::AppCategory::Travel as i32,
        Category::News => proto::AppCategory::News as i32,
        Category::Sports => proto::AppCategory::Sports as i32,
        Category::Shopping => proto::AppCategory::Shopping as i32,
        Category::FoodAndDrink => proto::AppCategory::FoodAndDrink as i32,
        Category::Music => proto::AppCategory::Music as i32,
        Category::Photography => proto::AppCategory::Photography as i32,
        Category::Utilities => proto::AppCategory::Utilities as i32,
        Category::Weather => proto::AppCategory::Weather as i32,
        Category::Games => proto::AppCategory::Games as i32,
        Category::Business => proto::AppCategory::Business as i32,
        Category::Communication => proto::AppCategory::Communication as i32,
        Category::Anime => proto::AppCategory::Anime as i32,
    }
}

/// True only for events whose `execution_mode` is explicitly `Remote`.
/// `event_type` (api / http / webhook / cron / …) describes the trigger
/// shape, not where the event runs — the user can mark any of those
/// Local and execute them on the desktop. Drop only events the user
/// has opted into running server-side.
fn is_remote_event(event: &proto::Event) -> bool {
    event
        .execution_mode
        .as_deref()
        .map(|m| m.eq_ignore_ascii_case("remote"))
        .unwrap_or(false)
}

/// Encode a proto message in the same format the rest of the
/// codebase writes to disk (prost-encode → lz4 with size prefix).
/// Reuses `flow_like::utils::compression::compress_to_file` against
/// a single-shot `InMemory` ObjectStore so the wire format stays in
/// lockstep with the on-disk format — the desktop's
/// `from_compressed::<proto::Board>(...)` decodes the returned bytes
/// round-trip without any custom wire format here.
async fn encode_proto<M: flow_like_types::Message + Default>(msg: &M) -> Result<Vec<u8>, ApiError> {
    use flow_like_storage::object_store::memory::InMemory;
    let store: Arc<dyn flow_like_storage::object_store::ObjectStore> = Arc::new(InMemory::new());
    let path = Path::from("blob");
    compress_to_file(store.clone(), path.clone(), msg)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("compress proto: {e}")))?;
    let bytes = store
        .get(&path)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("read encoded proto: {e}")))?
        .bytes()
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("read encoded proto bytes: {e}")))?;
    Ok(bytes.to_vec())
}

/// JSON counterpart of [`encode_proto`] — matches
/// `compress_to_file_json` (serde_json → lz4 with size prefix).
async fn encode_json(value: &flow_like_types::Value) -> Result<Vec<u8>, ApiError> {
    use flow_like_storage::object_store::memory::InMemory;
    let store: Arc<dyn flow_like_storage::object_store::ObjectStore> = Arc::new(InMemory::new());
    let path = Path::from("blob");
    compress_to_file_json(store.clone(), path.clone(), value)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("compress json: {e}")))?;
    let bytes = store
        .get(&path)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("read encoded json: {e}")))?
        .bytes()
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("read encoded json bytes: {e}")))?;
    Ok(bytes.to_vec())
}

/// Sanity check: scan the source app's content store for files that
/// look like meta-store artifacts. If any `.board` / `.event` /
/// `.template` / `.widget` / `.page` files turn up under
/// `apps/{src_app_id}/...` on the *content* store, that's a bug —
/// the deployment either has a misconfigured store split or a write
/// somewhere is using the wrong handle. Returns the offending paths
/// so callers can log + alert without aborting the fork.
pub async fn detect_meta_in_content_store(
    state: &AppState,
    src_app_id: &str,
) -> Result<Vec<String>, ApiError> {
    use futures_util::TryStreamExt;
    let credentials = state
        .master_credentials()
        .await
        .map_err(ApiError::internal_error)?;
    let content_store = credentials
        .to_store(false)
        .await
        .map_err(ApiError::internal_error)?
        .as_generic();
    let prefix = flow_like_storage::Path::from("apps").child(src_app_id.to_string());

    const META_SUFFIXES: &[&str] = &[".board", ".event", ".template", ".widget", ".page"];
    let mut leaks: Vec<String> = Vec::new();
    let mut listing = content_store.list(Some(&prefix));
    while let Some(item) = listing
        .try_next()
        .await
        .map_err(|e| ApiError::internal_error(flow_like_types::anyhow!("list src content: {e}")))?
    {
        let path_str = item.location.as_ref().to_string();
        if META_SUFFIXES.iter().any(|suf| path_str.ends_with(suf)) {
            leaks.push(path_str);
        }
    }
    Ok(leaks)
}

/// Typed entry point. Validates the option set and dispatches to the
/// concrete implementation. For now, only `OnlineSameStore` with a
/// `target_user_sub` is implemented (matches existing behavior); the
/// cross-store and offline-bundle modes return a clear error so callers
/// fail fast until later phases land.
pub async fn fork_with_options(
    state: &AppState,
    options: ForkOptions<'_>,
) -> Result<(String, ForkReport), ApiError> {
    match options.target_mode {
        ForkTarget::OnlineSameStore => {
            let user_sub = options.target_user_sub.ok_or_else(|| {
                ApiError::bad_request("online fork requires an authenticated user")
            })?;
            let visibility = options
                .requested_visibility
                .clone()
                .unwrap_or(flow_like::app::AppVisibility::Private);
            let (new_app_id, id_map, skipped) = fork_app_with_visibility(
                state,
                user_sub,
                options.source_app_id,
                options.language,
                options.remote_event_token,
                visibility,
            )
            .await?;
            Ok((
                new_app_id,
                ForkReport {
                    id_map,
                    skipped,
                    ..Default::default()
                },
            ))
        }
        ForkTarget::OfflineBundle => Err(ApiError::bad_request(
            "offline-bundle forks are not materialized via fork_with_options — call compute_offline_fork_bundle directly",
        )),
        ForkTarget::OnlineCrossStore => Err(ApiError::not_implemented(
            "cross-store online fork is not implemented yet",
        )),
    }
}

/// Forks a source app into a new user-owned copy with **all internal IDs
/// remapped** (boards, layers, nodes, pins, events, pages, widgets,
/// templates). Returns the destination app id, the mapping table, and a
/// list of items that were intentionally skipped (e.g. inaccessible
/// packages, OAuth-bound events that need re-auth) so the caller can
/// surface them to the end user.
///
/// `remote_event_token`, when supplied, is reused at every detected
/// remote-token site (HTTP `auth_token` in event config, `pat_encrypted`
/// on event sinks). OAuth tokens are always cleared and reported because
/// they cannot be substituted with a single PAT.
///
/// `dst_visibility` controls the destination app's visibility (defaults
/// to `Private` for online forks, `Offline` for offline-bundle forks
/// that the desktop will pick up via signed URL).
///
/// Online → online fork. Materializes the destination on the
/// server's storage (meta + content) and inserts every destination
/// DB row (App, roles, membership, events, pages, widgets,
/// templates, sinks, packages) inside one transaction. Used by the
/// course flow and by `POST /apps/{id}/fork`.
///
/// Offline-bundle forks **don't** go through this function —
/// they call `compute_offline_fork_bundle` directly, which doesn't
/// write a destination prefix or a DB row at all.
pub async fn fork_app_with_visibility(
    state: &AppState,
    user_sub: &str,
    src_app_id: &str,
    language: &str,
    remote_event_token: Option<&str>,
    dst_visibility: flow_like::app::AppVisibility,
) -> Result<(String, ForkIdMap, Vec<SkippedItem>), ApiError> {
    use crate::routes::app::events::db::db_model_to_event;
    use flow_like_types::ToProto;

    let new_app_id = create_id();
    let now = chrono::Utc::now().naive_utc();

    // Two physical stores per side:
    //   - meta: code-like state (manifest, boards, events, widgets,
    //     templates, pages, versioned forms)
    //   - content: user-controlled artifacts (metadata/, upload/, storage/)
    //
    // In some deployments these alias to the same bucket via
    // `with_default_store()`; in others they're separate buckets with
    // separate credentials. The fork must pick the right side per
    // resource — copying upload/ from `to_store(true)` (meta) silently
    // drops data when the stores are physically split.
    let src_credentials = state.master_credentials().await?;
    let src_meta_store = src_credentials.to_store(true).await?.as_generic();
    let src_content_store = src_credentials.to_store(false).await?.as_generic();

    let dst_credentials = state
        .scoped_credentials(user_sub, &new_app_id, CredentialsAccess::EditApp)
        .await?;
    let dst_meta_store = dst_credentials.to_store(true).await?.as_generic();
    let dst_content_store = dst_credentials.to_store(false).await?.as_generic();

    let src_prefix = Path::from("apps").child(src_app_id.to_string());
    let dst_prefix = Path::from("apps").child(new_app_id.clone());

    // ---- 1. Load the source manifest, overlay DB row -------------------
    // The manifest.app file on disk reflects state from the last
    // full-app save. Endpoints like `change_visibility`, `change_forking`,
    // `upsert_app` only touch the App DB row, so the file's
    // `visibility`, `version`, `execution_mode`, `allow_forking`, etc.
    // can be arbitrarily stale. Overlay the DB row's authoritative
    // values before remap so the fork ships current state.
    let manifest_path = src_prefix.child("manifest.app");
    let mut src_app_proto: proto::App =
        from_compressed(src_meta_store.clone(), manifest_path.clone())
            .await
            .map_err(|e| ApiError::internal_error(anyhow!("read source manifest: {e}")))?;
    overlay_app_row_into_manifest(state, src_app_id, &mut src_app_proto).await?;

    // Aggregate of items the caller chose / had to drop. Populated in
    // multiple stages (events, sinks, packages) and returned alongside
    // the id map. Pre-allocated so every detection site can `.push`.
    let mut skipped: Vec<SkippedItem> = Vec::new();

    // ---- 2. Read DB rows up front --------------------------------------
    // The DB is authoritative for app/meta/event/page/widget/template/role
    // rows — endpoints that flip a single field write only the DB row and
    // leave the manifest / `.event` / `.template` / `.widget` storage
    // files stale. Pull every row now so:
    //   1. ID allocation can union(manifest, DB) — a row that exists only
    //      in the DB still gets a fresh translated id and ships.
    //   2. Storage writes can use DB-derived data (events) instead of
    //      reading drift-prone storage files.
    //   3. The destination DB transaction has everything it needs without
    //      a second round of queries.
    //
    // Intentionally NOT copied (eligible callers for that data must come
    // from a fresh authoring action on the destination):
    //   - comments / ratings (Comment table cascades from App)
    //   - rating_sum / rating_count / avg_rating / relevance_score
    //   - download_count / interactions_count
    //   - per-day analytics rows (AppAnalyticsDaily)
    //   - sales / purchases / discounts (AppPurchase, AppSalesDaily, AppDiscount)
    //   - publication requests, invite links, invitations, notifications
    //   - role memberships (only the caller becomes a member, on owner role)
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

    // All roles defined on the source app — both the system roles
    // (Owner/Admin/the default member role) and any custom roles the
    // source-owner created. Copied faithfully (name, description,
    // permission bits, attributes) so workflows that branch on role
    // attributes keep working in the fork. Memberships are NOT copied
    // — only the caller becomes a member, on the destination's owner
    // role.
    let src_role_rows = role::Entity::find()
        .filter(role::Column::AppId.eq(src_app_id))
        .all(&state.db)
        .await?;

    let src_widget_rows = widget::Entity::find()
        .filter(widget::Column::AppId.eq(src_app_id))
        .all(&state.db)
        .await?;

    let src_template_rows = template::Entity::find()
        .filter(template::Column::AppId.eq(src_app_id))
        .all(&state.db)
        .await?;

    // Widgets and templates each have their own polymorphic Meta rows
    // (`Meta.widgetId` / `Meta.templateId`) — these rows have
    // `appId = NULL`, so the app-scoped query above misses them. The
    // widget listing endpoint (`GET /apps/{id}/widgets`) inner-joins
    // Meta and silently drops widgets without a Meta row, which means
    // copied widgets disappear from the destination's UI even though
    // their `.widget` blob and `Widget` row exist. Pull them now and
    // mirror them into the destination's id space inside the txn.
    let src_widget_id_list: Vec<String> = src_widget_rows.iter().map(|r| r.id.clone()).collect();
    let src_widget_meta_rows = if src_widget_id_list.is_empty() {
        Vec::new()
    } else {
        meta::Entity::find()
            .filter(meta::Column::WidgetId.is_in(src_widget_id_list.clone()))
            .all(&state.db)
            .await?
    };
    let src_template_id_list: Vec<String> =
        src_template_rows.iter().map(|r| r.id.clone()).collect();
    let src_template_meta_rows = if src_template_id_list.is_empty() {
        Vec::new()
    } else {
        meta::Entity::find()
            .filter(meta::Column::TemplateId.is_in(src_template_id_list.clone()))
            .all(&state.db)
            .await?
    };

    let src_sink_rows = event_sink::Entity::find()
        .filter(event_sink::Column::AppId.eq(src_app_id))
        .all(&state.db)
        .await?;

    // ---- 3. Pre-allocate top-level ID mappings ------------------------
    // Allocate from union(manifest, DB) so a row that exists only in
    // the DB still gets a fresh translated id. Boards have no DB row,
    // so they stay manifest-driven.
    let mut maps = ForkIdMap {
        source_app_id: src_app_id.to_string(),
        app_id: new_app_id.clone(),
        ..Default::default()
    };
    for b in &src_app_proto.boards {
        maps.boards.insert(b.clone(), create_id());
    }
    for e in src_app_proto
        .events
        .iter()
        .chain(src_event_rows.iter().map(|r| &r.id))
    {
        maps.events.entry(e.clone()).or_insert_with(create_id);
    }
    for p in src_app_proto
        .page_ids
        .iter()
        .chain(src_page_rows.iter().map(|r| &r.id))
    {
        maps.pages.entry(p.clone()).or_insert_with(create_id);
    }
    for w in src_app_proto
        .widget_ids
        .iter()
        .chain(src_widget_rows.iter().map(|r| &r.id))
    {
        maps.widgets.entry(w.clone()).or_insert_with(create_id);
    }
    for t in src_app_proto
        .templates
        .iter()
        .chain(src_template_rows.iter().map(|r| &r.id))
    {
        maps.templates.entry(t.clone()).or_insert_with(create_id);
    }
    for r in &src_role_rows {
        maps.roles.entry(r.id.clone()).or_insert_with(create_id);
    }

    // ---- 3. Load + remap each board, then save under new prefix -------
    let mut new_board_protos: Vec<(String, proto::Board)> = Vec::new();
    for src_board_id in &src_app_proto.boards {
        let board_path = src_prefix.child(format!("{}.board", src_board_id));
        let board_proto: proto::Board =
            match from_compressed::<proto::Board>(src_meta_store.clone(), board_path).await {
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
        compress_to_file(dst_meta_store.clone(), board_path, board)
            .await
            .map_err(|e| ApiError::internal_error(anyhow!("write board: {e}")))?;
    }

    // Pages: DB-driven so we never miss a row whose `.page` file lives at
    // an unexpected location. Source data may sit at the legacy
    // app-level JSON path (`apps/{app}/{page_id}.page`) written by the
    // removed `App::save_page`, so the read tries both. Writes go only
    // to the canonical board-scoped binary path
    // (`apps/{app}/_{board_id}/{page_id}.page`).
    fork_pages_db_driven(
        &src_meta_store,
        &dst_meta_store,
        &src_prefix,
        &dst_prefix,
        &src_page_rows,
        &maps,
    )
    .await?;

    // ---- 4. Events: DB-driven, remap, rewrite, write ------------------
    // Source events come from the DB rather than from `apps/{src}/events/`
    // on storage. The DB is authoritative — endpoints that flip a flag
    // or change a schedule write only the DB row, leaving the `.event`
    // file stale. Listing storage would ship that drift.
    //
    // The same rewritten config bytes are reused for the destination's
    // DB row (in the txn below) so the storage and DB views agree. We
    // collect those bytes here keyed by source event id.
    //
    // While we're at it, collect every (board_id, version) tuple any
    // event (or its canary) points at, so we can copy only those
    // versioned board files in step 4d below — versions not pointed to
    // are intentionally NOT copied (forks are seeded from the live
    // board, not from the version archive).
    let dst_events_dir = dst_prefix.child("events");
    let mut pointed_board_versions: std::collections::HashSet<(String, (u32, u32, u32))> =
        std::collections::HashSet::new();
    let mut rewritten_event_configs: HashMap<String, Vec<u8>> = HashMap::new();
    for row in &src_event_rows {
        let src_event_id = row.id.clone();
        let core_event = match db_model_to_event(row.clone()) {
            Ok(e) => e,
            Err(err) => {
                tracing::warn!("skip event {} (db→core conversion): {}", src_event_id, err);
                continue;
            }
        };
        let mut event_proto = core_event.to_proto();

        if let Some(v) = event_proto.board_version.as_ref() {
            pointed_board_versions
                .insert((event_proto.board_id.clone(), (v.major, v.minor, v.patch)));
        }
        if let Some(canary) = event_proto.canary.as_ref()
            && let Some(v) = canary.board_version.as_ref()
        {
            pointed_board_versions.insert((canary.board_id.clone(), (v.major, v.minor, v.patch)));
        }

        // Rewrite the HTTP `auth_token` in event.config: HTTP/api/webhook
        // events embed an auth token in the JSON config blob the trigger
        // checks against. We never carry the source's value into a fork —
        // either replace with the caller-supplied `remote_event_token`,
        // or clear it (and emit a Skipped entry so the UI can prompt
        // the user to set one in the fork settings).
        if matches!(event_proto.event_type.as_str(), "api" | "http" | "webhook")
            && let Some(rewritten) =
                rewrite_auth_token_in_config(&event_proto.config, remote_event_token)
        {
            event_proto.config = rewritten.bytes.clone();
            rewritten_event_configs.insert(src_event_id.clone(), rewritten.bytes);
            if rewritten.had_token && remote_event_token.is_none() {
                skipped.push(SkippedItem {
                    kind: SkippedKind::RemoteEvent,
                    source_id: src_event_id.clone(),
                    reason: format!(
                        "HTTP auth_token cleared on event {} — supply a token at fork time or set one in the fork's event settings",
                        src_event_id
                    ),
                });
            }
        }

        remap_event(&mut event_proto, &maps);
        let new_event_id = maps.translate_event(&src_event_id);
        let dst_event_path = dst_events_dir.child(format!("{}.event", new_event_id));
        compress_to_file(dst_meta_store.clone(), dst_event_path, &event_proto)
            .await
            .map_err(|e| ApiError::internal_error(anyhow!("write event: {e}")))?;
    }

    // ---- 4d. Copy versioned boards pointed to by events / canaries ---
    // For each (src_board_id, version) referenced above, load the
    // archived board, remap it (sharing the same `maps` so node/pin ids
    // line up with the live board), and save under the destination
    // versions tree. Version tuple stays the same — we don't re-bump.
    for (src_board_id, version) in &pointed_board_versions {
        let dst_board_id = maps.translate_board(src_board_id);
        let src_path = src_prefix
            .child("versions")
            .child(src_board_id.clone())
            .child(format!("{}_{}_{}.board", version.0, version.1, version.2));
        let board_proto: proto::Board =
            match from_compressed::<proto::Board>(src_meta_store.clone(), src_path).await {
                Ok(b) => b,
                Err(err) => {
                    tracing::warn!(
                        "skip versioned board {} v{}.{}.{}: {}",
                        src_board_id,
                        version.0,
                        version.1,
                        version.2,
                        err
                    );
                    continue;
                }
            };
        let mut remapped = remap_board(board_proto, &mut maps);
        // remap_board allocates a fresh board.id; force it back to the
        // destination live-board id so the archive stays addressable.
        remapped.id = dst_board_id.clone();
        let dst_path = dst_prefix
            .child("versions")
            .child(dst_board_id)
            .child(format!("{}_{}_{}.board", version.0, version.1, version.2));
        compress_to_file(dst_meta_store.clone(), dst_path, &remapped)
            .await
            .map_err(|e| ApiError::internal_error(anyhow!("write versioned board: {e}")))?;
    }

    // ---- 4b. Widgets: load JSON, rewrite ID, save under new prefix ----
    // Widgets are JSON (not proto). Their *internal* ids (component ids,
    // action ids) are widget-scoped and don't need cross-app translation;
    // only the top-level widget id is rewritten. Page-level WidgetInstance
    // bindings (event_id / page_id) live inside `Page` JSON, not here.
    fork_widgets(
        &src_meta_store,
        &dst_meta_store,
        &src_prefix,
        &dst_prefix,
        &maps,
    )
    .await?;

    // ---- 4c. Templates: load proto::Board, run remap, save -----------
    // Templates *are* boards on disk (`{template_id}.template` is a
    // serialized proto::Board), so we run the same `remap_board` pass
    // we use for live boards. The template id rewrite is layered on top.
    fork_templates(
        &src_meta_store,
        &dst_meta_store,
        &src_prefix,
        &dst_prefix,
        &mut maps,
    )
    .await?;

    // ---- 5/6. Copy content-store files --------------------------------
    // Mirror metadata/ + upload/ + storage/ from src content → dst
    // content. Both stores live under `apps/{id}/` so the prefix
    // translation is just the id substitution.
    copy_metadata_with_translation(
        &src_content_store,
        &dst_content_store,
        &src_prefix.child("metadata"),
        &dst_prefix.child("metadata"),
        &maps,
    )
    .await?;

    copy_object_prefix(
        &src_content_store,
        &dst_content_store,
        &src_prefix.child("upload"),
        &dst_prefix.child("upload"),
        "upload storage",
    )
    .await?;
    copy_object_prefix(
        &src_content_store,
        &dst_content_store,
        &src_prefix.child("storage"),
        &dst_prefix.child("storage"),
        "app storage",
    )
    .await?;

    // ---- 7. Rewrite the manifest --------------------------------------
    // The manifest's id lists are rebuilt from union(manifest, DB) and
    // deduped — endpoints like `upsert_event` / `upsert_widget` write
    // both manifest and DB row, but downstream flag-flips touch only
    // the DB. Without this union, rows that drifted out of the manifest
    // would be invisible to the desktop / `App::load` even though
    // `compress_to_file` and the txn already wrote them.
    src_app_proto.id = new_app_id.clone();
    src_app_proto.boards = src_app_proto
        .boards
        .iter()
        .map(|b| maps.translate_board(b))
        .collect();
    src_app_proto.events = dedupe_translated(
        src_app_proto
            .events
            .iter()
            .cloned()
            .chain(src_event_rows.iter().map(|r| r.id.clone())),
        |id| maps.translate_event(&id),
    );
    src_app_proto.page_ids = dedupe_translated(
        src_app_proto
            .page_ids
            .iter()
            .cloned()
            .chain(src_page_rows.iter().map(|r| r.id.clone())),
        |id| maps.translate_page(&id),
    );
    src_app_proto.widget_ids = dedupe_translated(
        src_app_proto
            .widget_ids
            .iter()
            .cloned()
            .chain(src_widget_rows.iter().map(|r| r.id.clone())),
        |id| translate_in_map(&maps.widgets, &id),
    );
    src_app_proto.templates = dedupe_translated(
        src_app_proto
            .templates
            .iter()
            .cloned()
            .chain(src_template_rows.iter().map(|r| r.id.clone())),
        |id| translate_in_map(&maps.templates, &id),
    );
    // Routes are authoritatively stored on the Event row's `route`
    // column — the manifest's `route_mappings` map is a denormalized
    // copy that endpoints don't always keep in sync. Rebuild it from
    // the source events that actually carry routes; that way the
    // destination's manifest reflects current state regardless of
    // whether the source manifest had drifted.
    let mut new_routes = HashMap::new();
    for e in &src_event_rows {
        if let Some(path) = e.route.as_ref() {
            new_routes.insert(path.clone(), maps.translate_event(&e.id));
        }
    }
    for (path, event_id) in src_app_proto.route_mappings.iter() {
        new_routes
            .entry(path.clone())
            .or_insert_with(|| maps.translate_event(event_id));
    }
    src_app_proto.route_mappings = new_routes;
    src_app_proto.visibility = app_visibility_to_proto(&dst_visibility);
    src_app_proto.status = proto::AppStatus::Active as i32;
    // Lineage — every fork carries the source app id so the UI can show
    // "forked from" and so we can later compute fork trees.
    src_app_proto.forked_from = Some(src_app_id.to_string());
    src_app_proto.forked_at = Some(flow_like_types::Timestamp::from(
        std::time::SystemTime::now(),
    ));
    src_app_proto.allow_forking = Some(false);
    // Bits are carried verbatim — bit ids are global registry ids, not
    // app-scoped, so a fork's bits set is just a clone of the source.
    // (`src_app_proto.bits` already has the right value; no change needed.)
    // Counters reset.
    src_app_proto.rating_sum = 0;
    src_app_proto.rating_count = 0;
    src_app_proto.download_count = 0;
    src_app_proto.interaction_count = 0;
    src_app_proto.avg_rating = None;
    src_app_proto.relevance_score = None;

    compress_to_file(
        dst_meta_store.clone(),
        dst_prefix.child("manifest.app"),
        &src_app_proto,
    )
    .await
    .map_err(|e| ApiError::internal_error(anyhow!("write manifest: {e}")))?;

    // ---- 8. DB transaction: app row, meta, roles, membership, events --
    // All source rows were fetched at the top of the function so
    // ID allocation and storage writes could use them. The txn below
    // just inserts the destination versions.
    let src_owner_role_id = src_app_row.owner_role_id.clone();
    let src_default_role_id = src_app_row.default_role_id.clone();
    let roles_to_copy: Vec<role::Model> = src_role_rows;

    // Filter packages: only carry packages the destination owner can
    // actually use. Anything public+free is always carried; private and
    // paid packages survive only if the target user has explicit access
    // (member, author, or owns a completed purchase). Inaccessible
    // packages get a SkippedItem entry so the UI can prompt the user
    // ("3 packages weren't copied because you don't have access").
    let (allowed_packages, package_skips) =
        filter_accessible_packages(state, user_sub, &src_package_rows).await?;

    // Translate sink rows: rewrite event_id to the destination id space,
    // re-encrypt PAT with the caller-supplied token (if any), clear
    // OAuth tokens (caller must re-auth on the fork). We compute the
    // destination ActiveModels here so the txn closure stays tight.
    let (sinks_to_insert, sink_skips) = prepare_dst_sinks(
        &src_sink_rows,
        &maps.events,
        remote_event_token,
        &state.encryption_key,
        &new_app_id,
        now,
    );

    let new_app_id_db = new_app_id.clone();
    let user_sub_owned = user_sub.to_string();
    let language_owned = language.to_string();
    let maps_arc = Arc::new(maps.clone());
    let dst_visibility_db = app_visibility_to_db(&dst_visibility);

    state
        .db
        .transaction::<_, (), DbErr>(|txn| {
            Box::pin(async move {
                let new_app_model = app::ActiveModel {
                    id: Set(new_app_id_db.clone()),
                    status: Set(Status::Active),
                    visibility: Set(dst_visibility_db),
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
                    allow_forking: Set(false),
                    forked_from: Set(Some(src_app_row.id.clone())),
                    forked_at: Set(Some(now)),
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

                // Faithfully copy every source role, preserving name,
                // description, permission bits, and attributes. IDs are
                // remapped via `maps_arc.roles` (pre-allocated outside
                // the txn).
                for r in &roles_to_copy {
                    let new_role_id = maps_arc.roles.get(&r.id).cloned().unwrap_or_else(create_id);
                    let new_role = role::ActiveModel {
                        id: Set(new_role_id),
                        name: Set(r.name.clone()),
                        description: Set(r.description.clone()),
                        permissions: Set(r.permissions),
                        app_id: Set(Some(new_app_id_db.clone())),
                        attributes: Set(r.attributes.clone()),
                        created_at: Set(now),
                        updated_at: Set(now),
                    };
                    new_role.insert(txn).await?;
                }

                // Resolve the destination's owner + default-member
                // role ids by translating the source's pointers
                // through the role map. We only trust the map when
                // the corresponding role was actually inserted above;
                // a fresh id pointing at nothing would FK-violate the
                // App.ownerRoleId / App.defaultRoleId constraints.
                let dst_owner_role_id = match src_owner_role_id
                    .as_deref()
                    .and_then(|src_id| maps_arc.roles.get(src_id).cloned())
                {
                    Some(id) => id,
                    None => {
                        // Source had no owner role recorded, or its
                        // pointer was stale. Invent one so the
                        // destination is at least valid; caller
                        // becomes the owner regardless.
                        let synthetic_id = create_id();
                        let synthetic = role::ActiveModel {
                            id: Set(synthetic_id.clone()),
                            name: Set("Owner".to_string()),
                            description: Set(Some("Owner role".to_string())),
                            permissions: Set(RolePermissions::Owner.bits()),
                            app_id: Set(Some(new_app_id_db.clone())),
                            attributes: NotSet,
                            created_at: Set(now),
                            updated_at: Set(now),
                        };
                        synthetic.insert(txn).await?;
                        synthetic_id
                    }
                };
                // For the default role, an unresolved pointer just
                // becomes None (the column is nullable). Better to
                // ship without a default-member role than synthesize
                // one with permissions the source never had.
                let dst_default_role_id = src_default_role_id
                    .as_deref()
                    .and_then(|src_id| maps_arc.roles.get(src_id).cloned());

                let mut app_active = inserted_app.into_active_model();
                app_active.owner_role_id = Set(Some(dst_owner_role_id.clone()));
                app_active.default_role_id = Set(dst_default_role_id);
                app_active.update(txn).await?;

                let owner_membership_id = create_id();
                let mship = membership::ActiveModel {
                    id: Set(owner_membership_id.clone()),
                    user_id: Set(user_sub_owned.clone()),
                    app_id: Set(new_app_id_db.clone()),
                    role_id: Set(dst_owner_role_id),
                    joined_via: NotSet,
                    created_at: Set(now),
                    updated_at: Set(now),
                };
                mship.insert(txn).await?;

                for pkg in &allowed_packages {
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

                    // If the event's config had its HTTP auth_token
                    // rewritten for the storage `.event` blob, mirror
                    // the same bytes into the DB row so the storage
                    // and DB views stay in sync. Re-wrap as the
                    // base64 JsonBinary shape `db_model_to_event`
                    // expects on read.
                    let new_config = match rewritten_event_configs.get(&e.id) {
                        Some(bytes) => {
                            use base64::Engine as _;
                            let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
                            Some(serde_json::json!({ "base64": b64 }))
                        }
                        None => e.config.clone(),
                    };

                    // Event input rows store the publish-time pin id of
                    // every input pin on the originating node. The ids
                    // need to land on the destination's pin id space —
                    // otherwise pre-run / trigger payloads use source
                    // pin ids that no longer exist on the fork. The
                    // proto-side blob in `compress_to_file` already
                    // remapped via `remap_event`; mirror the same
                    // translation onto the DB JSON column.
                    let new_inputs = translate_event_inputs_json(&e.inputs, &maps_arc);

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
                        config: Set(new_config),
                        inputs: Set(new_inputs),
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

                for w in &src_widget_rows {
                    let new_widget_id = maps_arc
                        .widgets
                        .get(&w.id)
                        .cloned()
                        .unwrap_or_else(create_id);
                    let new_widget = widget::ActiveModel {
                        id: Set(new_widget_id),
                        app_id: Set(new_app_id_db.clone()),
                        version: Set(w.version.clone()),
                        created_at: Set(now),
                        updated_at: Set(now),
                    };
                    new_widget.insert(txn).await?;
                }

                // Widget Meta rows: remap `widget_id`, keep payload.
                // Without these, `GET /apps/{id}/widgets` returns an
                // empty list because the join on Meta drops widgets
                // without a row.
                for m in &src_widget_meta_rows {
                    let dst_widget_id = m
                        .widget_id
                        .as_ref()
                        .and_then(|src_id| maps_arc.widgets.get(src_id).cloned());
                    if dst_widget_id.is_none() {
                        continue;
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
                        app_id: Set(None),
                        bit_id: Set(None),
                        course_id: Set(None),
                        template_id: Set(None),
                        widget_id: Set(dst_widget_id),
                        wasm_package_id: Set(None),
                        created_at: Set(now),
                        updated_at: Set(now),
                    };
                    new_meta.insert(txn).await?;
                }

                for t in &src_template_rows {
                    let new_template_id = maps_arc
                        .templates
                        .get(&t.id)
                        .cloned()
                        .unwrap_or_else(create_id);
                    let new_template = template::ActiveModel {
                        id: Set(new_template_id),
                        app_id: Set(new_app_id_db.clone()),
                        changelog: Set(t.changelog.clone()),
                        rating_sum: Set(0),
                        rating_count: Set(0),
                        version: Set(t.version.clone()),
                        created_at: Set(now),
                        updated_at: Set(now),
                    };
                    new_template.insert(txn).await?;
                }

                // Template Meta rows: same shape as widget meta — Meta
                // is keyed off `templateId` with `appId = NULL`, so the
                // app-scoped query couldn't see them. Without these,
                // `GET /apps/{id}/templates` returns an empty list.
                for m in &src_template_meta_rows {
                    let dst_template_id = m
                        .template_id
                        .as_ref()
                        .and_then(|src_id| maps_arc.templates.get(src_id).cloned());
                    if dst_template_id.is_none() {
                        continue;
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
                        app_id: Set(None),
                        bit_id: Set(None),
                        course_id: Set(None),
                        template_id: Set(dst_template_id),
                        widget_id: Set(None),
                        wasm_package_id: Set(None),
                        created_at: Set(now),
                        updated_at: Set(now),
                    };
                    new_meta.insert(txn).await?;
                }

                for sink in sinks_to_insert {
                    sink.insert(txn).await?;
                }

                Ok(())
            })
        })
        .await
        .map_err(|e| match e {
            sea_orm::TransactionError::Connection(err) => ApiError::from(err),
            sea_orm::TransactionError::Transaction(err) => ApiError::from(err),
        })?;

    skipped.extend(package_skips);
    skipped.extend(sink_skips);
    Ok((new_app_id, maps, skipped))
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

    // First pass: build node + pin + layer id maps for this board.
    // Layers must be registered BEFORE any node is rewritten — nodes that
    // live inside a function/collapsed layer are stored in `board.nodes`
    // with `node.layer = Some(layer_id)`; if the layer isn't in
    // `maps.layers` at rewrite time, `node.layer` would be cleared to
    // None, orphaning the node from its function and emptying the layer
    // when the desktop reconstructs `layer.nodes` from `node.layer`.
    register_node_pin_ids(&board.nodes, maps);
    for layer in board.layers.values() {
        maps.layers
            .entry(layer.id.clone())
            .or_insert_with(create_id);
        if let Some(parent) = layer.parent_id.as_ref() {
            maps.layers.entry(parent.clone()).or_insert_with(create_id);
        }
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
            .get(&layer.id)
            .cloned()
            .unwrap_or_else(create_id);
        layer.id = new_layer_id.clone();
        if let Some(parent) = layer.parent_id.as_ref() {
            layer.parent_id = maps.layers.get(parent).cloned();
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

    strip_board_secrets(&mut board);
    board
}

/// Clears `default_value` on every variable marked `secret = true`, both at
/// board level and inside each layer. Secrets must never travel into a
/// fork — even when the caller is the source-app owner — because the
/// destination may live in a different security boundary (different org,
/// different deployment, anonymous public download).
fn strip_board_secrets(board: &mut proto::Board) {
    for var in board.variables.values_mut() {
        if var.secret {
            var.default_value.clear();
        }
    }
    for layer in board.layers.values_mut() {
        for var in layer.variables.values_mut() {
            if var.secret {
                var.default_value.clear();
            }
        }
    }
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
        // Layers were pre-registered in remap_board, so a missing entry
        // means a stale pointer; preserve the original so the desktop
        // can surface the reference rather than silently orphan the node.
        node.layer = Some(maps.layers.get(layer).cloned().unwrap_or(layer.clone()));
    }
    // Agent / Call Reference style nodes carry a list of function-target
    // node ids in `fn_refs.fn_refs`. These are global node ids; without
    // translation, the destination would point at the source's nodes.
    if let Some(fn_refs) = node.fn_refs.as_mut() {
        fn_refs.fn_refs = fn_refs
            .fn_refs
            .iter()
            .map(|id| maps.nodes.get(id).cloned().unwrap_or(id.clone()))
            .collect();
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
    // Pin default values frequently encode a target id chosen by the
    // user — Call Function holds a layer id in `function_layer_id`,
    // Call Reference holds a node id in `fn_ref`, Goto / page-link
    // nodes hold page or event ids. Translate every JSON string we
    // recognize so those references land on the destination's id space.
    rewrite_default_value_ids(&mut pin.default_value, maps);
}

/// Walks a JSON-encoded pin `default_value` and rewrites every string
/// whose contents match a known source id (node, layer, event, page,
/// pin) to the destination id from `maps`. Strings that don't match
/// anything in the maps are left untouched. Empty bytes / non-JSON
/// payloads are no-ops so non-string defaults (numbers, structs that
/// don't reference ids) keep working.
fn rewrite_default_value_ids(default_value: &mut Vec<u8>, maps: &ForkIdMap) {
    if default_value.is_empty() {
        return;
    }
    let mut value: flow_like_types::Value = match serde_json::from_slice(default_value) {
        Ok(v) => v,
        Err(_) => return,
    };
    if !translate_ids_in_json(&mut value, maps) {
        return;
    }
    if let Ok(bytes) = serde_json::to_vec(&value) {
        *default_value = bytes;
    }
}

/// Recursively visits a JSON value and rewrites any string equal to a
/// known source id. Returns whether anything changed so callers can
/// skip a re-encode when the payload is untouched.
fn translate_ids_in_json(value: &mut flow_like_types::Value, maps: &ForkIdMap) -> bool {
    match value {
        flow_like_types::Value::String(s) => {
            if let Some(translated) = lookup_id(s, maps) {
                *s = translated;
                true
            } else {
                false
            }
        }
        flow_like_types::Value::Array(items) => {
            let mut changed = false;
            for item in items.iter_mut() {
                if translate_ids_in_json(item, maps) {
                    changed = true;
                }
            }
            changed
        }
        flow_like_types::Value::Object(map) => {
            let mut changed = false;
            for (_k, v) in map.iter_mut() {
                if translate_ids_in_json(v, maps) {
                    changed = true;
                }
            }
            changed
        }
        _ => false,
    }
}

fn lookup_id(src: &str, maps: &ForkIdMap) -> Option<String> {
    maps.nodes
        .get(src)
        .or_else(|| maps.layers.get(src))
        .or_else(|| maps.events.get(src))
        .or_else(|| maps.pages.get(src))
        .or_else(|| maps.pins.get(src))
        .or_else(|| maps.boards.get(src))
        .cloned()
}

/// Translates the `id` field of every entry in a serialized
/// `Vec<EventInput>` from source pin ids to destination pin ids.
/// Returns the input verbatim when it is `None`, empty, or doesn't
/// deserialize cleanly — the publish path validates schema, so a
/// blob that doesn't parse here is tolerated rather than dropped.
fn translate_event_inputs_json(
    inputs: &Option<flow_like_types::Value>,
    maps: &ForkIdMap,
) -> Option<flow_like_types::Value> {
    let mut value = inputs.as_ref()?.clone();
    let Some(items) = value.as_array_mut() else {
        return Some(value);
    };
    for item in items.iter_mut() {
        let Some(obj) = item.as_object_mut() else {
            continue;
        };
        if let Some(id_value) = obj.get_mut("id")
            && let Some(src_id) = id_value.as_str()
            && let Some(dst_id) = maps.pins.get(src_id).cloned()
        {
            *id_value = flow_like_types::Value::String(dst_id);
        }
    }
    Some(value)
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

    strip_event_secrets(event);
}

/// Clears `default_value` on every secret-marked variable inside an event
/// proto, including the canary's variables. The event's `config` bytes
/// are intentionally NOT touched here — token sites (HTTP auth_token,
/// PAT, OAuth) are replaced in Phase 4 with caller-supplied values.
fn strip_event_secrets(event: &mut proto::Event) {
    for var in event.variables.values_mut() {
        if var.secret {
            var.default_value.clear();
        }
    }
    if let Some(canary) = event.canary.as_mut() {
        for var in canary.variables.values_mut() {
            if var.secret {
                var.default_value.clear();
            }
        }
    }
}

/// Bounded concurrency for the copy loops. AWS S3 returns 503 SlowDown
/// at very high request rates per prefix; 32 is a generally safe ceiling
/// that still keeps a 1-GB fork under O(seconds) when CopyObject is in
/// play (server-side copies don't move bytes through the client).
const COPY_CONCURRENCY: usize = 32;

/// Copy a single object using a native server-side `copy()` when
/// available (S3 `CopyObject`, GCS `rewriteObject`, Azure `Copy Blob`,
/// local file copy), falling back to a streamed get→put when
/// `src_store.copy` errors out — typical for cross-bucket /
/// cross-deployment forks where the source and destination live in
/// different physical stores.
async fn copy_one(
    src_store: &Arc<dyn flow_like_storage::object_store::ObjectStore>,
    dst_store: &Arc<dyn flow_like_storage::object_store::ObjectStore>,
    src_path: &Path,
    dst_path: &Path,
    label: &str,
) -> Result<(), ApiError> {
    if std::ptr::eq(
        Arc::as_ptr(src_store) as *const (),
        Arc::as_ptr(dst_store) as *const (),
    ) {
        // Same Arc instance: definitely the same store. Native copy.
        return src_store
            .copy(src_path, dst_path)
            .await
            .map_err(|e| ApiError::internal_error(anyhow!("copy {label}: {e}")));
    }

    // Different Arcs: try native copy on the source store first (which
    // typically holds master credentials with read on src and write on
    // dst within the same bucket — same-deployment forks land here and
    // get server-side CopyObject for free). If that fails — almost
    // always because the dst path isn't in the source store's bucket
    // (cross-store fork) — fall back to streaming the bytes through.
    if src_store.copy(src_path, dst_path).await.is_ok() {
        return Ok(());
    }

    let bytes = src_store
        .get(src_path)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("read {label}: {e}")))?
        .bytes()
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("read {label} bytes: {e}")))?;
    dst_store
        .put(dst_path, bytes.into())
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("write {label}: {e}")))?;
    Ok(())
}

async fn copy_object_prefix(
    src_store: &Arc<dyn flow_like_storage::object_store::ObjectStore>,
    dst_store: &Arc<dyn flow_like_storage::object_store::ObjectStore>,
    src_prefix: &Path,
    dst_prefix: &Path,
    label: &str,
) -> Result<(), ApiError> {
    use futures::StreamExt;

    let mut listing = src_store.list(Some(src_prefix));
    let mut entries: Vec<(Path, Path)> = Vec::new();
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
        entries.push((item.location, dst_path));
    }

    let results: Vec<Result<(), ApiError>> =
        futures::stream::iter(entries.into_iter().map(|(src_path, dst_path)| async move {
            copy_one(src_store, dst_store, &src_path, &dst_path, label).await
        }))
        .buffer_unordered(COPY_CONCURRENCY)
        .collect()
        .await;
    for r in results {
        r?;
    }
    Ok(())
}

/// Read a page's content. Tries the canonical board-scoped binary
/// path first (`apps/{src}/_{board_id}/{page_id}.page`), then falls
/// back to the legacy app-level JSON path (`apps/{src}/{page_id}.page`)
/// written by the removed `App::save_page`. Source apps that pre-date
/// the storage unification still have data at the legacy location;
/// keeping the fallback here guarantees the fork sees their content
/// even before any backfill has run. Returns `None` if neither
/// location has a readable file; the caller logs and continues.
async fn read_source_page(
    src_store: &Arc<dyn flow_like_storage::object_store::ObjectStore>,
    src_prefix: &Path,
    src_board_id: Option<&str>,
    src_page_id: &str,
) -> Option<proto::Page> {
    let app_level = src_prefix.child(format!("{}.page", src_page_id));
    if let Ok(page) =
        from_compressed_json::<flow_like::a2ui::widget::Page>(src_store.clone(), app_level).await
    {
        return Some(page.into());
    }

    if let Some(board_id) = src_board_id {
        let board_level = src_prefix
            .child(format!("_{}", board_id))
            .child(format!("{}.page", src_page_id));
        if let Ok(page) = from_compressed::<proto::Page>(src_store.clone(), board_level).await {
            return Some(page);
        }
    }

    None
}

/// Apply the fork's id translations to a page in-place. Covers
/// every cross-app reference that lives anywhere inside the page
/// payload:
///
/// * `id`, `board_id` — top-level page metadata.
/// * `on_load_event_id` / `on_unload_event_id` / `on_interval_event_id`
///   — despite the name, these are **node** ids (a node id from an
///   `events_simple` node, per the `.proto` comment), not event row
///   ids. They translate via `maps.translate_node`.
/// * `content[].WidgetInstance` — the `widget_id` reference points
///   at a widget that just got reforked; the `action_bindings` map
///   carries `workflow_event_id` (node id) and `page_id` references
///   that must follow the new id space.
/// * `widget_refs` (per-page snapshot of widget definitions) — the
///   embedded `Widget.id` is rewritten so the page resolves against
///   the destination's widget id space.
///
/// Without these passes, a forked app's pages still load but every
/// "on click run this workflow / navigate to this page / show this
/// widget" hook silently points at the source app's ids.
fn remap_page(page: &mut proto::Page, new_page_id: &str, maps: &ForkIdMap) {
    page.id = new_page_id.to_string();
    if let Some(b) = page.board_id.as_ref() {
        page.board_id = Some(maps.translate_board(b));
    }
    if let Some(n) = page.on_load_event_id.as_ref() {
        page.on_load_event_id = Some(maps.translate_node(n));
    }
    if let Some(n) = page.on_unload_event_id.as_ref() {
        page.on_unload_event_id = Some(maps.translate_node(n));
    }
    if let Some(n) = page.on_interval_event_id.as_ref() {
        page.on_interval_event_id = Some(maps.translate_node(n));
    }

    for content in page.content.iter_mut() {
        match content.content_type.as_mut() {
            Some(proto::page_content::ContentType::Widget(instance)) => {
                remap_widget_instance(instance, maps);
            }
            Some(proto::page_content::ContentType::Component(comp)) => {
                remap_component_blob(comp, maps);
            }
            _ => {}
        }
    }

    for comp in page.components.iter_mut() {
        remap_component_blob(comp, maps);
    }

    for widget_def in page.widget_refs.values_mut() {
        if let Some(new_id) = maps.widgets.get(&widget_def.id) {
            widget_def.id = new_id.clone();
        }
        for comp in widget_def.components.iter_mut() {
            remap_component_blob(comp, maps);
        }
    }
}

/// Most component data lives as opaque JSON bytes inside
/// `proto::Component.component_json` — see `From<SurfaceComponent>` in
/// `protobuf/a2ui.rs`, which serializes `SurfaceComponent.component`
/// (a `serde_json::Value`) into those bytes and leaves the typed
/// `component` oneof unset. That's where buttons store their
/// `on_click` Action with `name = "workflow_event"` and a context map
/// holding `nodeId` / `boardId` / `appId` BoundValues, and where
/// `actionBindings` entries land in the runtime's
/// `{ workflow: { flowId } }` form. Without a JSON-level rewrite, a
/// fork's buttons keep firing the source app's nodes.
///
/// The walker is deliberately defensive: it only swaps a string when
/// the source value is present in the corresponding id map, so
/// non-id fields that happen to share a key name (e.g. a custom
/// `nodeId` inside a piece of user-authored JSON unrelated to a
/// workflow Action) are left alone unless they actually match a
/// known id.
fn remap_component_blob(comp: &mut proto::Component, maps: &ForkIdMap) {
    let Some(bytes) = comp.component_json.as_mut() else {
        return;
    };
    let mut value: flow_like_types::Value = match flow_like_types::json::from_slice(bytes) {
        Ok(v) => v,
        Err(err) => {
            tracing::warn!(
                "skip component_json remap for component {}: parse failed: {err}",
                comp.id
            );
            return;
        }
    };
    walk_remap_action_refs(&mut value, maps);
    match flow_like_types::json::to_vec(&value) {
        Ok(new_bytes) => *bytes = new_bytes,
        Err(err) => {
            tracing::warn!(
                "skip component_json remap for component {}: re-encode failed: {err}",
                comp.id
            );
        }
    }
}

/// Walk a JSON tree and remap every cross-app reference we recognize
/// by **field name**. Components embed cross-app ids in lots of
/// places besides the canonical `workflow_event` action — image
/// hotspots, dialogue choices, modal/drawer triggers, link routes,
/// custom user-authored `actions[].context` shapes, runtime
/// `{ workflow: { flowId } }` bindings, etc. Pattern-matching every
/// case explicitly was lossy. Instead, translate any value carried
/// under a known id-bearing key, regardless of where in the tree it
/// appears.
///
/// Field names recognized (both `camelCase` and `snake_case`):
///
/// * `nodeId` / `flowId` → `maps.nodes` (events_simple node ids)
/// * `boardId` → `maps.boards`
/// * `pageId` → `maps.pages`
/// * `widgetId` → `maps.widgets`
/// * `eventId` → `maps.events`
/// * `appId` → source-app-id → destination-app-id
///
/// Each value is accepted as either a bare string or a
/// `{ "literalString": "..." }` BoundValue wrapper. Translation only
/// fires when the embedded source string is actually present in the
/// corresponding map — so a user-authored field that happens to
/// share a name (e.g. a `nodeId` in arbitrary game state) is left
/// alone unless it actually matches a known source id, and a value
/// already on the destination's id space (e.g. after a typed remap
/// pass) is a no-op since it won't be in the map keys.
fn walk_remap_action_refs(value: &mut flow_like_types::Value, maps: &ForkIdMap) {
    match value {
        flow_like_types::Value::Object(map) => {
            for (key, val) in map.iter_mut() {
                match key.as_str() {
                    "nodeId" | "node_id" | "flowId" | "flow_id" => {
                        translate_bound_string(Some(val), &maps.nodes);
                    }
                    "boardId" | "board_id" => {
                        translate_bound_string(Some(val), &maps.boards);
                    }
                    "pageId" | "page_id" => {
                        translate_bound_string(Some(val), &maps.pages);
                    }
                    "widgetId" | "widget_id" => {
                        translate_bound_string(Some(val), &maps.widgets);
                    }
                    "eventId" | "event_id" => {
                        translate_bound_string(Some(val), &maps.events);
                    }
                    "appId" | "app_id" => {
                        translate_app_id(Some(val), maps);
                    }
                    _ => {}
                }
                walk_remap_action_refs(val, maps);
            }
        }
        flow_like_types::Value::Array(arr) => {
            for v in arr.iter_mut() {
                walk_remap_action_refs(v, maps);
            }
        }
        _ => {}
    }
}

/// A `BoundValue` is either a bare string (rare in practice) or
/// `{ "literalString": "<id>" }` / `{ "literalNumber": ... }` /
/// `{ "literalBool": ... }` / `{ "path": "..." }`. Only the literal
/// string form can hold an id reference, so that's the only one we
/// rewrite. Path bindings resolve at runtime against the data model
/// and are not cross-app refs.
fn translate_bound_string(
    target: Option<&mut flow_like_types::Value>,
    mapping: &HashMap<String, String>,
) {
    let Some(target) = target else { return };
    match target {
        flow_like_types::Value::String(s) => {
            if let Some(new_id) = mapping.get(s.as_str()) {
                *s = new_id.clone();
            }
        }
        flow_like_types::Value::Object(obj) => {
            if let Some(flow_like_types::Value::String(s)) = obj.get_mut("literalString") {
                if let Some(new_id) = mapping.get(s.as_str()) {
                    *s = new_id.clone();
                }
            }
        }
        _ => {}
    }
}

/// AppId rewrite uses the fork's source→destination app id pair
/// rather than a generic map. We translate only when the embedded
/// value matches the known source app id, so unrelated `appId`
/// strings the user might have authored elsewhere stay untouched.
fn translate_app_id(target: Option<&mut flow_like_types::Value>, maps: &ForkIdMap) {
    let Some(target) = target else { return };
    let src = maps.source_app_id.as_str();
    let dst = maps.app_id.clone();
    if src.is_empty() || dst.is_empty() {
        return;
    }
    match target {
        flow_like_types::Value::String(s) => {
            if s == src {
                *s = dst;
            }
        }
        flow_like_types::Value::Object(obj) => {
            if let Some(flow_like_types::Value::String(s)) = obj.get_mut("literalString") {
                if s == src {
                    *s = dst;
                }
            }
        }
        _ => {}
    }
}

/// Translate every cross-app reference on a `WidgetInstance`. The
/// instance's `widget_id` lookup follows the widget id map, and each
/// `ActionBinding` rewrites the `workflow_event_id` (node id) and
/// `page_id` arms so the destination's runtime fires the right
/// node / navigates to the right page. URL and custom-action arms
/// are pure data and are left as-is.
fn remap_widget_instance(instance: &mut proto::WidgetInstance, maps: &ForkIdMap) {
    if let Some(new_id) = maps.widgets.get(&instance.widget_id) {
        instance.widget_id = new_id.clone();
    }
    if let Some(widget_ref) = instance.widget_ref.as_mut() {
        if let Some(new_id) = maps.widgets.get(&widget_ref.widget_id) {
            widget_ref.widget_id = new_id.clone();
        }
    }
    for binding in instance.action_bindings.values_mut() {
        if let Some(binding_type) = binding.binding_type.as_mut() {
            match binding_type {
                proto::action_binding::BindingType::WorkflowEventId(id) => {
                    *id = maps.translate_node(id);
                }
                proto::action_binding::BindingType::PageId(id) => {
                    *id = maps.translate_page(id);
                }
                proto::action_binding::BindingType::ExternalUrl(_) => {}
                proto::action_binding::BindingType::CustomAction(_) => {}
            }
        }
    }
}

/// Write a remapped page to the canonical board-scoped layout
/// (`_{board_id}/{page_id}.page`, compressed binary `proto::Page`) —
/// the unified storage location now used by both API and desktop.
/// Pages without a board id can't be persisted (they have no place
/// on disk under the unified scheme), so we skip them with a warning.
async fn write_destination_page(
    dst_store: &Arc<dyn flow_like_storage::object_store::ObjectStore>,
    dst_prefix: &Path,
    dst_board_id: Option<&str>,
    page_proto: &proto::Page,
) -> Result<(), ApiError> {
    let Some(board_id) = dst_board_id else {
        tracing::warn!(
            "skip writing page {} on destination: no board_id (unreachable under unified storage)",
            page_proto.id
        );
        return Ok(());
    };
    let board_level = dst_prefix
        .child(format!("_{}", board_id))
        .child(format!("{}.page", page_proto.id));
    compress_to_file(dst_store.clone(), board_level, page_proto)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("write page: {e}")))?;
    Ok(())
}

/// DB-driven page mirroring used by the online fork. Iterates the
/// authoritative `Page` rows for the source app, reads each page from
/// whichever storage convention has it, remaps ids, and writes to
/// both destination conventions.
async fn fork_pages_db_driven(
    src_store: &Arc<dyn flow_like_storage::object_store::ObjectStore>,
    dst_store: &Arc<dyn flow_like_storage::object_store::ObjectStore>,
    src_prefix: &Path,
    dst_prefix: &Path,
    src_page_rows: &[page::Model],
    maps: &ForkIdMap,
) -> Result<(), ApiError> {
    for row in src_page_rows {
        let src_page_id = row.id.clone();
        let new_page_id = maps.translate_page(&src_page_id);
        let new_board_id = row.board_id.as_ref().map(|b| maps.translate_board(b));

        let mut page_proto = match read_source_page(
            src_store,
            src_prefix,
            row.board_id.as_deref(),
            &src_page_id,
        )
        .await
        {
            Some(p) => p,
            None => {
                tracing::warn!(
                    "skip page {} during fork: no readable source file at app-level or board-scoped path",
                    src_page_id
                );
                continue;
            }
        };

        remap_page(&mut page_proto, &new_page_id, maps);
        write_destination_page(dst_store, dst_prefix, new_board_id.as_deref(), &page_proto).await?;
    }
    Ok(())
}

fn translate_in_map(map: &HashMap<String, String>, src: &str) -> String {
    map.get(src).cloned().unwrap_or_else(|| src.to_string())
}

/// Translates an iterator of source ids through a translation closure
/// and collects the destination ids in deterministic insertion order
/// without duplicates. Used to rebuild the manifest's `events`,
/// `page_ids`, `widget_ids`, and `templates` arrays from
/// `union(manifest, DB rows)` — the manifest order is preserved while
/// rows that drifted out of it still get appended.
fn dedupe_translated<I, F>(src_ids: I, translate: F) -> Vec<String>
where
    I: IntoIterator<Item = String>,
    F: Fn(String) -> String,
{
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut out: Vec<String> = Vec::new();
    for id in src_ids {
        let dst = translate(id);
        if seen.insert(dst.clone()) {
            out.push(dst);
        }
    }
    out
}

fn app_visibility_to_proto(v: &flow_like::app::AppVisibility) -> i32 {
    match v {
        flow_like::app::AppVisibility::Public => proto::AppVisibility::Public as i32,
        flow_like::app::AppVisibility::PublicRequestAccess => {
            proto::AppVisibility::PublicRequestAccess as i32
        }
        flow_like::app::AppVisibility::Private => proto::AppVisibility::Private as i32,
        flow_like::app::AppVisibility::Prototype => proto::AppVisibility::Prototype as i32,
        flow_like::app::AppVisibility::Offline => proto::AppVisibility::Offline as i32,
    }
}

fn app_visibility_to_db(v: &flow_like::app::AppVisibility) -> Visibility {
    match v {
        flow_like::app::AppVisibility::Public => Visibility::Public,
        flow_like::app::AppVisibility::PublicRequestAccess => Visibility::PublicRequestAccess,
        flow_like::app::AppVisibility::Private => Visibility::Private,
        flow_like::app::AppVisibility::Prototype => Visibility::Prototype,
        flow_like::app::AppVisibility::Offline => Visibility::Offline,
    }
}

/// Loads each widget JSON listed at the source app prefix and writes it
/// under the destination prefix with a fresh top-level id. Widget bodies
/// (components, actions) live in widget scope and don't reference cross-app
/// resources, so only the top-level `id` field is rewritten.
///
/// Widget metadata is handled by `copy_metadata_with_translation` — this
/// pass only touches the `{widget_id}.widget` JSON files.
async fn fork_widgets(
    src_store: &Arc<dyn flow_like_storage::object_store::ObjectStore>,
    dst_store: &Arc<dyn flow_like_storage::object_store::ObjectStore>,
    src_prefix: &Path,
    dst_prefix: &Path,
    maps: &ForkIdMap,
) -> Result<(), ApiError> {
    for (src_widget_id, new_widget_id) in &maps.widgets {
        let src_path = src_prefix.child(format!("{}.widget", src_widget_id));
        let mut widget: flow_like_types::Value =
            match from_compressed_json(src_store.clone(), src_path).await {
                Ok(w) => w,
                Err(err) => {
                    tracing::warn!("skip widget {}: {}", src_widget_id, err);
                    continue;
                }
            };
        if let Some(obj) = widget.as_object_mut() {
            obj.insert(
                "id".to_string(),
                flow_like_types::Value::String(new_widget_id.clone()),
            );
        }
        // Components inside the widget def carry the same `Action` /
        // `actionBindings` shapes that pages do, so we run the same
        // remap pass over the widget JSON. Without this, a button or
        // similar element placed *inside* a widget keeps firing the
        // source app's nodes after fork.
        walk_remap_action_refs(&mut widget, maps);
        let dst_path = dst_prefix.child(format!("{}.widget", new_widget_id));
        compress_to_file_json(dst_store.clone(), dst_path, &widget)
            .await
            .map_err(|e| ApiError::internal_error(anyhow!("write widget: {e}")))?;
    }
    Ok(())
}

/// Loads each template (a serialized `proto::Board`) and runs it through
/// the same `remap_board` pass we use for live boards. The on-disk file
/// is renamed to the destination template id, but the file format and
/// internal node/pin/layer structure are otherwise identical.
async fn fork_templates(
    src_store: &Arc<dyn flow_like_storage::object_store::ObjectStore>,
    dst_store: &Arc<dyn flow_like_storage::object_store::ObjectStore>,
    src_prefix: &Path,
    dst_prefix: &Path,
    maps: &mut ForkIdMap,
) -> Result<(), ApiError> {
    let template_pairs: Vec<(String, String)> = maps
        .templates
        .iter()
        .map(|(s, d)| (s.clone(), d.clone()))
        .collect();
    for (src_template_id, new_template_id) in template_pairs {
        let src_path = src_prefix.child(format!("{}.template", src_template_id));
        let board_proto: proto::Board =
            match from_compressed::<proto::Board>(src_store.clone(), src_path).await {
                Ok(b) => b,
                Err(err) => {
                    tracing::warn!("skip template {}: {}", src_template_id, err);
                    continue;
                }
            };
        // remap_board allocates fresh node/pin/layer ids inside the
        // template; since the template body is otherwise self-contained,
        // this keeps internal references consistent without colliding
        // with live-board ids.
        let mut remapped = remap_board(board_proto, maps);
        // remap_board rewrote board.id to a fresh id; force it back to
        // the chosen template id for path consistency.
        remapped.id = new_template_id.clone();
        let dst_path = dst_prefix.child(format!("{}.template", new_template_id));
        compress_to_file(dst_store.clone(), dst_path, &remapped)
            .await
            .map_err(|e| ApiError::internal_error(anyhow!("write template: {e}")))?;
    }
    Ok(())
}

/// Copies the `metadata/` subtree from src to dst, rewriting any path
/// segment that names a `widget_id`, `template_id`, or `page_id` so it
/// matches the destination's id space. App-level files (e.g. `metadata/
/// {lang}.meta`) are copied verbatim.
async fn copy_metadata_with_translation(
    src_store: &Arc<dyn flow_like_storage::object_store::ObjectStore>,
    dst_store: &Arc<dyn flow_like_storage::object_store::ObjectStore>,
    src_meta_dir: &Path,
    dst_meta_dir: &Path,
    maps: &ForkIdMap,
) -> Result<(), ApiError> {
    use futures::StreamExt;

    let mut listing = src_store.list(Some(src_meta_dir));
    let mut entries: Vec<(Path, Path)> = Vec::new();
    while let Some(item) = listing
        .try_next()
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("list metadata: {e}")))?
    {
        let path_str = item.location.as_ref().to_string();
        let Some(suffix) = path_str.strip_prefix(src_meta_dir.as_ref()) else {
            continue;
        };
        let suffix = suffix.trim_start_matches('/');
        if suffix.is_empty() {
            continue;
        }

        // Translate the second path segment when the first is a known
        // category. e.g. `widgets/{src_id}/{lang}.meta` becomes
        // `widgets/{dst_id}/{lang}.meta`.
        let translated_suffix = match suffix.split('/').collect::<Vec<_>>().as_slice() {
            ["widgets", id, rest @ ..] => {
                Some(translated_metadata_path("widgets", id, rest, &maps.widgets))
            }
            ["templates", id, rest @ ..] => Some(translated_metadata_path(
                "templates",
                id,
                rest,
                &maps.templates,
            )),
            ["pages", id, rest @ ..] => {
                Some(translated_metadata_path("pages", id, rest, &maps.pages))
            }
            _ => None,
        };
        let dst_suffix = translated_suffix.unwrap_or_else(|| suffix.to_string());
        let dst_path = dst_meta_dir.child(dst_suffix);
        entries.push((item.location, dst_path));
    }

    let results: Vec<Result<(), ApiError>> =
        futures::stream::iter(entries.into_iter().map(|(src_path, dst_path)| async move {
            copy_one(src_store, dst_store, &src_path, &dst_path, "metadata").await
        }))
        .buffer_unordered(COPY_CONCURRENCY)
        .collect()
        .await;
    for r in results {
        r?;
    }
    Ok(())
}

fn translated_metadata_path(
    category: &str,
    src_id: &str,
    rest: &[&str],
    map: &HashMap<String, String>,
) -> String {
    let dst_id = translate_in_map(map, src_id);
    if rest.is_empty() {
        format!("{}/{}", category, dst_id)
    } else {
        format!("{}/{}/{}", category, dst_id, rest.join("/"))
    }
}

struct RewrittenConfig {
    bytes: Vec<u8>,
    had_token: bool,
}

/// Rewrites the `auth_token` field inside a JSON-encoded `event.config`
/// blob. Returns `None` if the config is empty / not JSON-shaped (in
/// which case the caller should leave it alone). When the source had a
/// token and `replacement` is `Some`, the new token replaces it; when
/// `replacement` is `None`, the field is removed (set to empty string)
/// so the destination ships with no live secret.
fn rewrite_auth_token_in_config(
    config: &[u8],
    replacement: Option<&str>,
) -> Option<RewrittenConfig> {
    if config.is_empty() {
        return None;
    }
    let mut value: flow_like_types::Value = serde_json::from_slice(config).ok()?;
    let obj = value.as_object_mut()?;
    let had_token = obj
        .get("auth_token")
        .and_then(|v| v.as_str())
        .map(|s| !s.is_empty())
        .unwrap_or(false);
    if !had_token && replacement.is_none() {
        return None;
    }
    let new_value = match replacement {
        Some(token) => flow_like_types::Value::String(token.to_string()),
        None => flow_like_types::Value::String(String::new()),
    };
    obj.insert("auth_token".to_string(), new_value);
    let bytes = serde_json::to_vec(&value).ok()?;
    Some(RewrittenConfig { bytes, had_token })
}

/// Builds destination `EventSink` rows from the source sinks, with
/// translated `event_id`, re-encrypted PAT (when the caller supplied a
/// token), cleared OAuth tokens, and the destination app id. Returns a
/// list of sinks ready to insert and a list of `SkippedItem`s describing
/// what couldn't be carried as-is (PATs without a replacement token,
/// OAuth bindings that need re-auth).
fn prepare_dst_sinks(
    src_sinks: &[event_sink::Model],
    event_id_map: &HashMap<String, String>,
    remote_event_token: Option<&str>,
    encryption_key: &[u8; 32],
    new_app_id: &str,
    now: chrono::NaiveDateTime,
) -> (Vec<event_sink::ActiveModel>, Vec<SkippedItem>) {
    use crate::routes::app::events::db::encrypt_token;

    let mut to_insert = Vec::with_capacity(src_sinks.len());
    let mut skipped = Vec::new();
    for src in src_sinks {
        let new_event_id = match event_id_map.get(&src.event_id) {
            Some(id) => id.clone(),
            None => {
                // Sink references an event that no longer exists in the
                // source manifest — skip; nothing to bind to.
                skipped.push(SkippedItem {
                    kind: SkippedKind::RemoteEvent,
                    source_id: src.id.clone(),
                    reason: format!(
                        "sink {} pointed at unknown event {} on source",
                        src.id, src.event_id
                    ),
                });
                continue;
            }
        };

        let new_pat = match (&src.pat_encrypted, remote_event_token) {
            (Some(_), Some(token)) => Some(encrypt_token(token, encryption_key)),
            (Some(_), None) => {
                skipped.push(SkippedItem {
                    kind: SkippedKind::RemoteEvent,
                    source_id: src.event_id.clone(),
                    reason: format!(
                        "PAT cleared on sink for event {} — supply a remote_event_token at fork time or re-bind in the fork's event settings",
                        src.event_id
                    ),
                });
                None
            }
            _ => None,
        };

        let new_auth_token = match (&src.auth_token, remote_event_token) {
            (Some(_), Some(token)) => Some(token.to_string()),
            (Some(_), None) => {
                // Already reported via the event.config-side skip above
                // — don't double-count, but clear the column.
                None
            }
            _ => None,
        };

        if src.oauth_tokens_encrypted.is_some() {
            skipped.push(SkippedItem {
                kind: SkippedKind::OAuthRequiresReauth,
                source_id: src.event_id.clone(),
                reason: format!(
                    "OAuth tokens for event {} were cleared — re-authenticate the provider on the fork before triggering the event",
                    src.event_id
                ),
            });
        }

        to_insert.push(event_sink::ActiveModel {
            id: Set(create_id()),
            event_id: Set(new_event_id),
            app_id: Set(new_app_id.to_string()),
            sink_type: Set(src.sink_type.clone()),
            active: Set(false),
            path: Set(src.path.clone()),
            auth_token: Set(new_auth_token),
            // Webhook secrets are also bearer-style secrets — clear and
            // let the user reset post-fork. Same rationale as PAT: don't
            // carry source's secret into a fork that may live in a
            // different security boundary.
            webhook_secret: Set(None),
            cron_expression: Set(src.cron_expression.clone()),
            cron_timezone: Set(src.cron_timezone.clone()),
            pat_encrypted: Set(new_pat),
            oauth_tokens_encrypted: Set(None),
            profile_json: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            method: Set(src.method.clone()),
        });
    }
    (to_insert, skipped)
}

/// Splits the source app's package list into a "carry into the fork"
/// vector and a "skipped" log. A package survives iff one of:
///
/// 1. It is public AND free (price <= 0); OR
/// 2. The target user is an author, member (WasmPackageUser row), or has
///    a completed purchase.
///
/// Packages that fail both checks are reported via `SkippedItem` so the
/// UI can prompt the user to install them manually after the fork.
async fn filter_accessible_packages(
    state: &AppState,
    user_sub: &str,
    src_packages: &[app_package::Model],
) -> Result<(Vec<app_package::Model>, Vec<SkippedItem>), ApiError> {
    let mut allowed = Vec::with_capacity(src_packages.len());
    let mut skipped = Vec::new();
    for pkg in src_packages {
        match is_package_accessible(state, user_sub, &pkg.package_id).await? {
            PackageAccess::PublicFree | PackageAccess::Granted => {
                allowed.push(pkg.clone());
            }
            PackageAccess::Denied(reason) => {
                skipped.push(SkippedItem {
                    kind: SkippedKind::Package,
                    source_id: pkg.package_id.clone(),
                    reason,
                });
            }
        }
    }
    Ok((allowed, skipped))
}

enum PackageAccess {
    PublicFree,
    Granted,
    Denied(String),
}

async fn is_package_accessible(
    state: &AppState,
    user_sub: &str,
    package_id: &str,
) -> Result<PackageAccess, ApiError> {
    let Some(pkg) = wasm_package::Entity::find_by_id(package_id)
        .one(&state.db)
        .await?
    else {
        return Ok(PackageAccess::Denied(format!(
            "package {} no longer exists in the registry",
            package_id
        )));
    };

    let is_public = matches!(
        pkg.visibility,
        WasmPackageVisibility::Public | WasmPackageVisibility::PublicRequestAccess
    );
    if matches!(pkg.visibility, WasmPackageVisibility::Public) && pkg.price <= 0 {
        return Ok(PackageAccess::PublicFree);
    }

    // Author?
    if wasm_package_author::Entity::find()
        .filter(wasm_package_author::Column::PackageId.eq(package_id))
        .filter(wasm_package_author::Column::UserId.eq(user_sub))
        .one(&state.db)
        .await?
        .is_some()
    {
        return Ok(PackageAccess::Granted);
    }

    // Granted member?
    if wasm_package_user::Entity::find()
        .filter(wasm_package_user::Column::PackageId.eq(package_id))
        .filter(wasm_package_user::Column::UserId.eq(user_sub))
        .one(&state.db)
        .await?
        .is_some()
    {
        return Ok(PackageAccess::Granted);
    }

    // Completed purchase?
    use crate::entity::sea_orm_active_enums::PurchaseStatus;
    if wasm_package_purchase::Entity::find()
        .filter(wasm_package_purchase::Column::PackageId.eq(package_id))
        .filter(wasm_package_purchase::Column::UserId.eq(user_sub))
        .filter(wasm_package_purchase::Column::Status.eq(PurchaseStatus::Completed))
        .one(&state.db)
        .await?
        .is_some()
    {
        return Ok(PackageAccess::Granted);
    }

    let reason = if is_public {
        format!(
            "package {} is paid (price {} cents) and you don't have a purchase on file",
            package_id, pkg.price
        )
    } else {
        format!(
            "package {} is private and you are not a member or author",
            package_id
        )
    };
    Ok(PackageAccess::Denied(reason))
}
