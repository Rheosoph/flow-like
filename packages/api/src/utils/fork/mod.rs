use crate::{
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
pub mod db_schema;
pub mod policy;
pub mod preview;
use flow_like::a2ui::{
    id_refs::{self, IdRef},
    page_remap,
};
use flow_like::utils::compression::{
    compress_to_file, compress_to_file_json, from_compressed, from_compressed_json,
};
use flow_like_storage::Path;
use flow_like_types::{anyhow, create_id, dispatch::ETAG_BOUND_LATEST_VERSION_SENTINEL, proto};
use futures_util::TryStreamExt;
pub use policy::{ForkDatabaseMode, ForkPolicy};
use sea_orm::{
    ActiveModelTrait,
    ActiveValue::{NotSet, Set},
    ColumnTrait, DbErr, EntityTrait, IntoActiveModel, QueryFilter, TransactionTrait,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
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
    /// Excluded by the source app owner's fork policy
    Policy,
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
/// user_sub, src_app_id, language)` signature.
///
/// `fork_with_options` performs **no permission checking**. The
/// `allow_forking` opt-in and the read-permission gate live at the
/// endpoint layer (`permission::fork_permission::check_can_fork`).
///
/// The source app owner's [`policy::ForkPolicy`] **is** enforced here, but
/// it is deliberately *not* an option: it is loaded from the source `App`
/// row inside the engine, so a caller can neither supply it nor have it
/// silently ignored.
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
}

impl<'a> ForkOptions<'a> {
    /// Default options matching the existing course-fork behavior:
    /// online same-store, no token, no anonymous caller.
    pub fn for_user(source_app_id: &'a str, user_sub: &'a str, language: &'a str) -> Self {
        Self {
            source_app_id,
            target_user_sub: Some(user_sub),
            target_mode: ForkTarget::OnlineSameStore,
            language,
            remote_event_token: None,
            requested_visibility: None,
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
/// (compressed proto for boards/events/templates/manifest/metadata,
/// compressed JSON for widgets/pages, and raw bytes for `media/...`
/// entries). `path` is relative to `apps/{new_app_id}/` so the desktop
/// can write it back at the same offset.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct MetaBlob {
    /// Relative path under `apps/{new_app_id}/`, slash-delimited.
    pub relative_path: String,
    /// Base64-encoded payload — exactly what would land on disk if
    /// the destination were materialized server-side.
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
    pub warnings: Vec<String>,
    pub blobs: Vec<MetaBlob>,
    /// The source owner's policy, so the desktop can show what it got.
    pub policy: ForkPolicy,
    /// Source-relative content prefixes the desktop must not mirror.
    /// Advisory — the caller already holds `ReadFiles` on the source, so
    /// this keeps the copy honest rather than protecting the bytes.
    pub content_exclude_prefixes: Vec<String>,
    /// Populated for schema-only database forks: the desktop creates
    /// these tables empty in its local project DB.
    pub db_table_schemas: Vec<db_schema::ForkTableSchema>,
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
/// 5. Encodes DB-backed app/widget/template metadata rows as local
///    `metadata/.../*.meta` files in the destination id space.
/// 6. Base64-encodes each remapped blob with its `apps/{new_app_id}/`-
///    relative path.
/// 7. Inlines app metadata media from `media/apps/{src_app_id}/...`
///    as raw `media/...` blobs. Online app media lives outside the
///    `apps/{id}` content prefix, while desktop readers expect it
///    under `apps/{id}/media`.
///
/// Storage-backed user content (`upload/`, `storage/`, and any legacy
/// `metadata/` files) is **not** in the bundle — `begin_offline_fork`
/// hands the desktop a single scoped `ReadAppContent` credential over
/// the source's content prefix and the desktop pulls those bytes
/// directly. Inline DB metadata wins if both sources provide the same
/// destination path.
pub async fn compute_offline_fork_bundle(
    state: &AppState,
    src_app_id: &str,
) -> Result<OfflineMetaBundle, ApiError> {
    use crate::routes::app::events::db::db_model_to_event;
    use base64::Engine as _;
    use flow_like_types::ToProto;

    let credentials = state.master_credentials().await?;
    let src_meta_store = credentials.to_store(true).await?.as_generic();
    let src_content_store = credentials.to_store(false).await?.as_generic();

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

    // Owner-defined policy, loaded server-side. The desktop is told what
    // it may pull, but the credential it receives still covers the whole
    // source content prefix — the forker already holds `ReadFiles` on the
    // source, so narrowing it would protect nothing.
    let src_app_row = app::Entity::find_by_id(src_app_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    let policy = ForkPolicy::from_app_row(&src_app_row);
    let mut warnings: Vec<String> = Vec::new();

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
    let mut shipped_boards: HashSet<String> = Default::default();
    let mut shipped_events: HashSet<String> = Default::default();
    let mut shipped_pages: HashSet<String> = Default::default();
    let mut shipped_widgets: HashSet<String> = Default::default();
    let mut shipped_templates: HashSet<String> = Default::default();

    // ---- 2. Boards: load + remap in memory -----------------------
    // Board files are the UI's source for page discovery
    // (`board.page_ids`), but that list can drift from the Page DB
    // rows. Reconcile it before remapping, then delay the write until
    // after page copying so the final board lists only pages that
    // actually made it into the bundle.
    let mut remapped_boards: Vec<(String, String, proto::Board)> = Vec::new();
    if !policy.flows {
        for src_board_id in &manifest_proto.boards {
            skipped.push(SkippedItem {
                kind: SkippedKind::Policy,
                source_id: src_board_id.clone(),
                reason: "flows are excluded by the source app's fork policy".to_string(),
            });
        }
        warnings.push(
            "This fork contains no flows, so it has no runnable logic — only the app shell."
                .to_string(),
        );
    }
    for src_board_id in manifest_proto
        .boards
        .clone()
        .iter()
        .filter(|_| policy.flows)
    {
        let board_path = src_prefix.child(format!("{}.board", src_board_id));
        let mut board_proto: proto::Board = match from_compressed::<proto::Board>(
            src_meta_store.clone(),
            board_path,
        )
        .await
        {
            Ok(b) => b,
            Err(err) => {
                tracing::warn!("skip board {} during offline bundle: {}", src_board_id, err);
                skipped.push(SkippedItem {
                        kind: SkippedKind::Other,
                        source_id: src_board_id.clone(),
                        reason: format!(
                            "board could not be read from the source app, so its flows, pages and events do not travel: {err}"
                        ),
                    });
                continue;
            }
        };
        overlay_board_page_ids_from_rows(&mut board_proto, src_board_id, &page_rows);
        let dst_board_id = maps.translate_board(src_board_id);
        let mut remapped = remap_board(board_proto, &mut maps);
        remapped.id = dst_board_id.clone();
        remapped_boards.push((src_board_id.clone(), dst_board_id, remapped));
        shipped_boards.insert(src_board_id.clone());
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
        let Some(src_board_id) = row.board_id.as_deref() else {
            tracing::warn!(
                "skip page {} in offline bundle: row has no board_id",
                src_page_id
            );
            skipped.push(SkippedItem {
                kind: SkippedKind::Other,
                source_id: src_page_id,
                reason: "page row names no board, so it has no place in the destination"
                    .to_string(),
            });
            continue;
        };
        let Some(dst_board_id) = maps
            .boards
            .get(src_board_id)
            .filter(|_| shipped_boards.contains(src_board_id))
            .cloned()
        else {
            tracing::warn!(
                "skip page {} in offline bundle: row points at unknown board {}",
                src_page_id,
                src_board_id
            );
            skipped.push(SkippedItem {
                kind: SkippedKind::Other,
                source_id: src_page_id,
                reason: format!(
                    "page points at board {} which was not shipped in the fork",
                    src_board_id
                ),
            });
            continue;
        };

        let mut page_proto = match read_source_page(
            &src_meta_store,
            &src_prefix,
            Some(src_board_id),
            row.id.as_str(),
        )
        .await
        {
            Some(p) => p,
            None => {
                tracing::warn!(
                    "skip page {} in offline bundle: no readable source file",
                    row.id
                );
                skipped.push(SkippedItem {
                    kind: SkippedKind::Other,
                    source_id: row.id.clone(),
                    reason: "page has no readable source file at either the app-level or the board-scoped path".to_string(),
                });
                continue;
            }
        };

        let issues = remap_page(&mut page_proto, &new_page_id, &maps);
        if !issues.is_empty() {
            skipped.push(SkippedItem {
                kind: SkippedKind::Other,
                source_id: row.id.clone(),
                reason: issues.reason("page"),
            });
        }

        let bytes = encode_proto(&page_proto).await?;
        blobs.push(MetaBlob {
            relative_path: format!("_{}/{}.page", dst_board_id, new_page_id),
            data_b64: base64::engine::general_purpose::STANDARD.encode(bytes),
        });
        shipped_pages.insert(row.id.clone());
    }

    let shipped_page_dst_ids: HashSet<String> = shipped_pages
        .iter()
        .map(|p| maps.translate_page(p))
        .collect();
    for (_src_board_id, dst_board_id, mut board) in remapped_boards {
        retain_shipped_board_pages(&mut board, &shipped_page_dst_ids);
        let bytes = encode_proto(&board).await?;
        blobs.push(MetaBlob {
            relative_path: format!("{}.board", dst_board_id),
            data_b64: base64::engine::general_purpose::STANDARD.encode(bytes),
        });
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
                skipped.push(SkippedItem {
                    kind: SkippedKind::Other,
                    source_id: src_event_id.clone(),
                    reason: format!("event row could not be read: {err}"),
                });
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

        if !guard_reserved_event_versions(&mut event_proto, &src_event_id, &mut skipped) {
            continue;
        }

        if !event_target_available(&event_proto, &shipped_boards, &maps) {
            skipped.push(SkippedItem {
                kind: SkippedKind::Other,
                source_id: src_event_id.clone(),
                reason: format!(
                    "event {} points at board/node that was not shipped in the fork",
                    src_event_id
                ),
            });
            continue;
        }
        if event_proto
            .canary
            .as_ref()
            .is_some_and(|canary| !canary_target_available(canary, &shipped_boards, &maps))
        {
            skipped.push(SkippedItem {
                kind: SkippedKind::Other,
                source_id: src_event_id.clone(),
                reason: format!(
                    "canary target for event {} pointed at a board/node that was not shipped and was cleared",
                    src_event_id
                ),
            });
            event_proto.canary = None;
        }
        drop_unavailable_variants(
            &mut event_proto,
            &src_event_id,
            &shipped_boards,
            &maps,
            &mut skipped,
        );

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
        for variant in &event_proto.variants {
            if let Some(v) = variant.board_version.as_ref() {
                pointed_board_versions
                    .insert((variant.board_id.clone(), (v.major, v.minor, v.patch)));
            }
        }

        if matches!(event_proto.event_type.as_str(), "api" | "http" | "webhook")
            && let Some(rewritten) = rewrite_auth_token_in_config(&event_proto.config, None)
        {
            event_proto.config = rewritten.bytes;
            if rewritten.had_token {
                skipped.push(SkippedItem {
                    kind: SkippedKind::RemoteEvent,
                    source_id: src_event_id.clone(),
                    reason: format!(
                        "HTTP auth_token cleared on event {} — set a new token in the fork's event settings if this local event should keep one",
                        src_event_id
                    ),
                });
            }
        }

        // Same guard as the online engine: `remap_event` translates
        // `default_page_id` unconditionally and nothing downstream
        // validates it, so drop the pointer when the page did not ship.
        if let Some(default_page) = event_proto.default_page_id.as_deref()
            && !shipped_pages.contains(default_page)
        {
            event_proto.default_page_id = None;
        }
        for variant in event_proto.variants.iter_mut() {
            if let Some(page) = variant.default_page_id.as_deref()
                && !shipped_pages.contains(page)
            {
                variant.default_page_id = None;
            }
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
        if is_reserved_etag_dispatch_version_tuple(*version) {
            skipped.push(SkippedItem {
                kind: SkippedKind::Other,
                source_id: src_board_id.clone(),
                reason: "the version reserved for ETag-bound Latest execution was not copied"
                    .to_string(),
            });
            continue;
        }
        let dst_board_id = maps.translate_board(src_board_id);
        let src_versioned_path = src_prefix
            .child("versions")
            .child(src_board_id.clone())
            .child(format!("{}_{}_{}.board", version.0, version.1, version.2));
        let board_proto: proto::Board = match from_compressed::<proto::Board>(
            src_meta_store.clone(),
            src_versioned_path,
        )
        .await
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
                skipped.push(SkippedItem {
                        kind: SkippedKind::Other,
                        source_id: src_board_id.clone(),
                        reason: format!(
                            "pinned board version {}.{}.{} could not be read, so events pinned to it have no board to run: {err}",
                            version.0, version.1, version.2
                        ),
                    });
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
    if !policy.widgets {
        for src_widget_id in maps.widgets.keys() {
            skipped.push(SkippedItem {
                kind: SkippedKind::Policy,
                source_id: src_widget_id.clone(),
                reason: "widgets are excluded by the source app's fork policy".to_string(),
            });
        }
        if !maps.widgets.is_empty() {
            warnings.push(
                "Widgets were not copied. Nodes that instantiate a widget by id will fail at run time."
                    .to_string(),
            );
        }
    }
    for (src_widget_id, new_widget_id) in maps.widgets.clone().iter().filter(|_| policy.widgets) {
        let src_path = src_prefix.child(format!("{}.widget", src_widget_id));
        let mut widget: flow_like_types::Value =
            match from_compressed_json(src_meta_store.clone(), src_path).await {
                Ok(w) => w,
                Err(err) => {
                    tracing::warn!("skip widget {}: {}", src_widget_id, err);
                    skipped.push(SkippedItem {
                        kind: SkippedKind::Other,
                        source_id: src_widget_id.clone(),
                        reason: format!(
                            "widget definition could not be read from the source app: {err}"
                        ),
                    });
                    continue;
                }
            };
        if let Some(obj) = widget.as_object_mut() {
            obj.insert(
                "id".to_string(),
                flow_like_types::Value::String(new_widget_id.clone()),
            );
        }
        for issue in remap_widget_json(&mut widget, &maps) {
            skipped.push(SkippedItem {
                kind: SkippedKind::Other,
                source_id: src_widget_id.clone(),
                reason: format!(
                    "widget kept a reference payload the fork could not rewrite: {issue}"
                ),
            });
        }
        let bytes = encode_json(&widget).await?;
        blobs.push(MetaBlob {
            relative_path: format!("{}.widget", new_widget_id),
            data_b64: base64::engine::general_purpose::STANDARD.encode(bytes),
        });
        shipped_widgets.insert(src_widget_id.clone());
    }

    // ---- 6. Templates (proto::Board) ------------------------------
    if !policy.templates {
        for src_template_id in maps.templates.keys() {
            skipped.push(SkippedItem {
                kind: SkippedKind::Policy,
                source_id: src_template_id.clone(),
                reason: "templates are excluded by the source app's fork policy".to_string(),
            });
        }
    }
    let template_pairs: Vec<(String, String)> = if policy.templates {
        maps.templates
            .iter()
            .map(|(s, d)| (s.clone(), d.clone()))
            .collect()
    } else {
        Vec::new()
    };
    for (src_template_id, new_template_id) in template_pairs {
        let template_page_ids =
            list_template_page_ids(&src_meta_store, &src_prefix, &src_template_id, &mut skipped)
                .await;
        for src_page_id in &template_page_ids {
            maps.pages
                .entry(src_page_id.clone())
                .or_insert_with(create_id);
        }
        let src_path = src_prefix.child(format!("{}.template", src_template_id));
        let board_proto: proto::Board =
            match from_compressed::<proto::Board>(src_meta_store.clone(), src_path).await {
                Ok(b) => b,
                Err(err) => {
                    tracing::warn!("skip template {}: {}", src_template_id, err);
                    skipped.push(SkippedItem {
                        kind: SkippedKind::Other,
                        source_id: src_template_id.clone(),
                        reason: format!(
                            "template board could not be read from the source app: {err}"
                        ),
                    });
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

        for template_page in read_template_pages(
            &src_meta_store,
            &src_prefix,
            &src_template_id,
            &template_page_ids,
            &maps,
            &mut skipped,
        )
        .await?
        {
            let bytes = encode_proto(&template_page).await?;
            blobs.push(MetaBlob {
                relative_path: format!("_template_{}/{}.page", new_template_id, template_page.id),
                data_b64: base64::engine::general_purpose::STANDARD.encode(bytes),
            });
        }

        shipped_templates.insert(src_template_id);
    }

    // ---- 7. Metadata rows: DB → local content-store files -----------
    append_metadata_blobs_from_db(
        state,
        src_app_id,
        &maps,
        &shipped_widgets,
        &shipped_templates,
        &mut blobs,
    )
    .await?;

    // ---- 7b. App metadata media: server content layout → desktop layout
    append_media_blobs_from_store(&src_content_store, src_app_id, &mut blobs).await?;

    // ---- 8. Rewrite + ship the manifest ----------------------------
    // Rebuild each id list as the union of (manifest, DB rows) so a
    // resource that exists only in the DB still ends up on the
    // desktop's manifest. Source ids are deduped via HashSet, then
    // translated through the id map. Boards have no DB row, so they
    // stay manifest-driven.
    manifest_proto.id = new_app_id.clone();
    manifest_proto.boards = manifest_proto
        .boards
        .iter()
        .filter(|b| shipped_boards.contains(*b))
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

    // Schema-only forks ship the source's Arrow schemas inline and tell
    // the desktop to skip those tables' objects. Reserved artifact tables
    // (`__x__`) carry Data Studio configuration and are mirrored whole, so
    // they are neither excluded nor recreated.
    let (db_table_schemas, skip_tables) = match policy.databases {
        ForkDatabaseMode::SchemaOnly => {
            let schemas = db_schema::read_project_db_schemas(state, src_app_id).await?;
            let names = schemas.iter().map(|s| s.table.clone()).collect::<Vec<_>>();
            if !names.is_empty() {
                warnings.push(format!(
                    "{} database table(s) arrive empty. Indices were not copied — rebuild them in Data Studio.",
                    names.len()
                ));
            }
            (schemas, names)
        }
        _ => (Vec::new(), Vec::new()),
    };
    let content_exclude_prefixes = policy::offline_content_exclude_prefixes(&policy, &skip_tables);
    if !policy.files {
        skipped.push(SkippedItem {
            kind: SkippedKind::Policy,
            source_id: "upload/".to_string(),
            reason: "uploaded files are excluded by the source app's fork policy".to_string(),
        });
        warnings.push(
            "Uploaded files were not copied. Flow paths pointing at them will not resolve in the fork."
                .to_string(),
        );
    }
    if policy.databases == ForkDatabaseMode::None {
        skipped.push(SkippedItem {
            kind: SkippedKind::Policy,
            source_id: "storage/db".to_string(),
            reason: "the project database is excluded by the source app's fork policy".to_string(),
        });
    }

    Ok(OfflineMetaBundle {
        new_app_id,
        id_map: maps,
        skipped,
        warnings,
        blobs,
        policy,
        content_exclude_prefixes,
        db_table_schemas,
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

fn is_reserved_etag_dispatch_version_tuple(version: (u32, u32, u32)) -> bool {
    version == ETAG_BOUND_LATEST_VERSION_SENTINEL
}

fn is_reserved_etag_dispatch_version(version: &proto::Version) -> bool {
    is_reserved_etag_dispatch_version_tuple((version.major, version.minor, version.patch))
}

fn guard_reserved_event_versions(
    event: &mut proto::Event,
    source_event_id: &str,
    skipped: &mut Vec<SkippedItem>,
) -> bool {
    if event
        .board_version
        .as_ref()
        .is_some_and(is_reserved_etag_dispatch_version)
    {
        skipped.push(SkippedItem {
            kind: SkippedKind::Other,
            source_id: source_event_id.to_string(),
            reason: format!(
                "event {} selects the version reserved for ETag-bound Latest execution and was not forked",
                source_event_id
            ),
        });
        return false;
    }
    if event
        .canary
        .as_ref()
        .and_then(|canary| canary.board_version.as_ref())
        .is_some_and(is_reserved_etag_dispatch_version)
    {
        skipped.push(SkippedItem {
            kind: SkippedKind::Other,
            source_id: source_event_id.to_string(),
            reason: format!(
                "canary target for event {} selected the version reserved for ETag-bound Latest execution and was cleared",
                source_event_id
            ),
        });
        event.canary = None;
    }
    event.variants.retain(|variant| {
        if !variant
            .board_version
            .as_ref()
            .is_some_and(is_reserved_etag_dispatch_version)
        {
            return true;
        }
        skipped.push(SkippedItem {
            kind: SkippedKind::Other,
            source_id: source_event_id.to_string(),
            reason: format!(
                "variant '{}' of event {} selected the version reserved for ETag-bound Latest execution and was dropped",
                variant.name, source_event_id
            ),
        });
        false
    });
    true
}

fn event_target_available(
    event: &proto::Event,
    shipped_boards: &HashSet<String>,
    maps: &ForkIdMap,
) -> bool {
    shipped_boards.contains(&event.board_id)
        && (event.node_id.is_empty() || maps.nodes.contains_key(&event.node_id))
}

fn canary_target_available(
    canary: &proto::Canary,
    shipped_boards: &HashSet<String>,
    maps: &ForkIdMap,
) -> bool {
    shipped_boards.contains(&canary.board_id)
        && (canary.node_id.is_empty() || maps.nodes.contains_key(&canary.node_id))
}

fn variant_target_available(
    variant: &proto::EventVariant,
    shipped_boards: &HashSet<String>,
    maps: &ForkIdMap,
) -> bool {
    shipped_boards.contains(&variant.board_id)
        && (variant.node_id.is_empty() || maps.nodes.contains_key(&variant.node_id))
}

/// Drop every variant whose board/node did not ship, one `SkippedItem` each.
/// Weights are NOT redistributed — a dropped variant's share simply falls
/// back to the primary target.
fn drop_unavailable_variants(
    event: &mut proto::Event,
    source_event_id: &str,
    shipped_boards: &HashSet<String>,
    maps: &ForkIdMap,
    skipped: &mut Vec<SkippedItem>,
) {
    event.variants.retain(|variant| {
        if variant_target_available(variant, shipped_boards, maps) {
            return true;
        }
        skipped.push(SkippedItem {
            kind: SkippedKind::Other,
            source_id: source_event_id.to_string(),
            reason: format!(
                "variant '{}' of event {} pointed at a board/node that was not shipped and was dropped",
                variant.name, source_event_id
            ),
        });
        false
    });
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

async fn append_metadata_blobs_from_db(
    state: &AppState,
    src_app_id: &str,
    maps: &ForkIdMap,
    shipped_widgets: &std::collections::HashSet<String>,
    shipped_templates: &std::collections::HashSet<String>,
    blobs: &mut Vec<MetaBlob>,
) -> Result<(), ApiError> {
    use base64::Engine as _;
    use flow_like::bit::Metadata;
    use flow_like_types::ToProto;

    let app_meta_rows = meta::Entity::find()
        .filter(meta::Column::AppId.eq(src_app_id))
        .all(&state.db)
        .await?;
    for row in app_meta_rows {
        push_metadata_blob(blobs, format!("metadata/{}.meta", row.lang), row).await?;
    }

    let widget_ids: Vec<String> = shipped_widgets.iter().cloned().collect();
    if !widget_ids.is_empty() {
        let widget_meta_rows = meta::Entity::find()
            .filter(meta::Column::WidgetId.is_in(widget_ids))
            .all(&state.db)
            .await?;
        for row in widget_meta_rows {
            let Some(src_id) = row.widget_id.as_deref() else {
                continue;
            };
            let Some(dst_id) = maps.widgets.get(src_id) else {
                continue;
            };
            push_metadata_blob(
                blobs,
                format!("metadata/widgets/{}/{}.meta", dst_id, row.lang),
                row,
            )
            .await?;
        }
    }

    let template_ids: Vec<String> = shipped_templates.iter().cloned().collect();
    if !template_ids.is_empty() {
        let template_meta_rows = meta::Entity::find()
            .filter(meta::Column::TemplateId.is_in(template_ids))
            .all(&state.db)
            .await?;
        for row in template_meta_rows {
            let Some(src_id) = row.template_id.as_deref() else {
                continue;
            };
            let Some(dst_id) = maps.templates.get(src_id) else {
                continue;
            };
            push_metadata_blob(
                blobs,
                format!("metadata/templates/{}/{}.meta", dst_id, row.lang),
                row,
            )
            .await?;
        }
    }

    async fn push_metadata_blob(
        blobs: &mut Vec<MetaBlob>,
        relative_path: String,
        row: meta::Model,
    ) -> Result<(), ApiError> {
        let metadata = Metadata::from(row);
        let proto_metadata = metadata.to_proto();
        let bytes = encode_proto(&proto_metadata).await?;
        blobs.push(MetaBlob {
            relative_path,
            data_b64: base64::engine::general_purpose::STANDARD.encode(bytes),
        });
        Ok(())
    }

    Ok(())
}

async fn append_media_blobs_from_store(
    src_store: &Arc<dyn flow_like_storage::object_store::ObjectStore>,
    src_app_id: &str,
    blobs: &mut Vec<MetaBlob>,
) -> Result<(), ApiError> {
    use base64::Engine as _;

    let src_media_dir = Path::from("media")
        .child("apps")
        .child(src_app_id.to_string());
    let src_media_dir_str = src_media_dir.as_ref().to_string();
    let mut listing = src_store.list(Some(&src_media_dir));

    while let Some(item) = listing
        .try_next()
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("list app metadata media: {e}")))?
    {
        let path_str = item.location.as_ref().to_string();
        let Some(relative_path) = relative_to_prefix(&path_str, &src_media_dir_str) else {
            continue;
        };
        if relative_path.is_empty() {
            continue;
        }

        let bytes = src_store
            .get(&item.location)
            .await
            .map_err(|e| ApiError::internal_error(anyhow!("read app metadata media: {e}")))?
            .bytes()
            .await
            .map_err(|e| ApiError::internal_error(anyhow!("read app metadata media bytes: {e}")))?;
        blobs.push(MetaBlob {
            relative_path: format!("media/{}", relative_path),
            data_b64: base64::engine::general_purpose::STANDARD.encode(bytes),
        });
    }

    Ok(())
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
            fork_app_with_visibility(
                state,
                user_sub,
                options.source_app_id,
                options.language,
                options.remote_event_token,
                visibility,
            )
            .await
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
) -> Result<(String, ForkReport), ApiError> {
    use crate::routes::app::events::db::{db_model_to_event, event_to_db_model};
    use flow_like_types::{FromProto, ToProto};

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

    let dst_credentials = state.master_credentials().await?;
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
    let mut warnings: Vec<String> = Vec::new();

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

    // The source owner decides what a fork of their app contains. Loaded
    // here rather than taken from the caller so it cannot be supplied and
    // then silently ignored. ID allocation below stays exhaustive: a
    // reference to an excluded artifact resolves to a destination id with
    // nothing behind it (a local dangling ref) rather than falling back to
    // the source id and pointing into someone else's app.
    let policy = ForkPolicy::from_app_row(&src_app_row);

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

    // ---- 3. Load + remap boards in memory ---------------------------
    // See the offline fork path above: reconcile board.page_ids from
    // Page DB rows, then wait until page copying finishes before
    // writing boards so stale board files cannot hide pages or point
    // at missing ones.
    // Boards are the structural root: `shipped_boards` gates pages, events
    // and the versioned-board archives, so excluding flows cascades through
    // the rest of the pipeline without any further branching.
    let mut new_board_protos: Vec<(String, String, proto::Board)> = Vec::new();
    let mut shipped_boards: HashSet<String> = Default::default();
    if !policy.flows {
        for src_board_id in &src_app_proto.boards {
            skipped.push(SkippedItem {
                kind: SkippedKind::Policy,
                source_id: src_board_id.clone(),
                reason: "flows are excluded by the source app's fork policy".to_string(),
            });
        }
        warnings.push(
            "This fork contains no flows, so it has no runnable logic — only the app shell."
                .to_string(),
        );
    }
    for src_board_id in src_app_proto.boards.iter().filter(|_| policy.flows) {
        let board_path = src_prefix.child(format!("{}.board", src_board_id));
        let mut board_proto: proto::Board = match from_compressed::<proto::Board>(
            src_meta_store.clone(),
            board_path,
        )
        .await
        {
            Ok(b) => b,
            Err(err) => {
                tracing::warn!("skipping board {} during fork: {}", src_board_id, err);
                skipped.push(SkippedItem {
                        kind: SkippedKind::Other,
                        source_id: src_board_id.clone(),
                        reason: format!(
                            "board could not be read from the source app, so its flows, pages and events do not travel: {err}"
                        ),
                    });
                continue;
            }
        };
        overlay_board_page_ids_from_rows(&mut board_proto, src_board_id, &src_page_rows);
        let new_board_id = maps.translate_board(src_board_id);
        let mut remapped = remap_board(board_proto, &mut maps);
        remapped.id = new_board_id.clone();
        new_board_protos.push((src_board_id.clone(), new_board_id, remapped));
        shipped_boards.insert(src_board_id.clone());
    }

    // Pages: DB-driven so we never miss a row whose `.page` file lives at
    // an unexpected location. Source data may sit at the legacy
    // app-level JSON path (`apps/{app}/{page_id}.page`) written by the
    // removed `App::save_page`, so the read tries both. Writes go only
    // to the canonical board-scoped binary path
    // (`apps/{app}/_{board_id}/{page_id}.page`).
    let shipped_pages = fork_pages_db_driven(
        &src_meta_store,
        &dst_meta_store,
        &src_prefix,
        &dst_prefix,
        &src_page_rows,
        &maps,
        &shipped_boards,
        &mut skipped,
    )
    .await?;

    let shipped_page_dst_ids: HashSet<String> = shipped_pages
        .iter()
        .map(|p| maps.translate_page(p))
        .collect();
    for (_src_board_id, new_board_id, mut board) in new_board_protos {
        retain_shipped_board_pages(&mut board, &shipped_page_dst_ids);
        let board_path = dst_prefix.child(format!("{}.board", new_board_id));
        compress_to_file(dst_meta_store.clone(), board_path, &board)
            .await
            .map_err(|e| ApiError::internal_error(anyhow!("write board: {e}")))?;
    }

    // ---- 4. Events: DB-driven, remap, rewrite, write ------------------
    // Source events come from the DB rather than from `apps/{src}/events/`
    // on storage. The DB is authoritative — endpoints that flip a flag
    // or change a schedule write only the DB row, leaving the `.event`
    // file stale. Listing storage would ship that drift.
    //
    // The same remapped event proto is reused for the destination's
    // DB row (in the txn below) so the storage and DB views agree:
    // board/node/page ids, input pin ids, canary ids, stripped
    // variables, and rewritten config all come from one source.
    //
    // While we're at it, collect every (board_id, version) tuple any
    // event (or its canary) points at, so we can copy only those
    // versioned board files in step 4d below — versions not pointed to
    // are intentionally NOT copied (forks are seeded from the live
    // board, not from the version archive).
    let dst_events_dir = dst_prefix.child("events");
    let mut pointed_board_versions: std::collections::HashSet<(String, (u32, u32, u32))> =
        std::collections::HashSet::new();
    let mut rewritten_events: HashMap<String, flow_like::flow::event::Event> = HashMap::new();
    for row in &src_event_rows {
        let src_event_id = row.id.clone();
        let core_event = match db_model_to_event(row.clone()) {
            Ok(e) => e,
            Err(err) => {
                tracing::warn!("skip event {} (db→core conversion): {}", src_event_id, err);
                skipped.push(SkippedItem {
                    kind: SkippedKind::Other,
                    source_id: src_event_id.clone(),
                    reason: format!("event row could not be read: {err}"),
                });
                continue;
            }
        };
        let mut event_proto = core_event.to_proto();

        if !guard_reserved_event_versions(&mut event_proto, &src_event_id, &mut skipped) {
            continue;
        }

        if !event_target_available(&event_proto, &shipped_boards, &maps) {
            skipped.push(SkippedItem {
                kind: SkippedKind::Other,
                source_id: src_event_id.clone(),
                reason: format!(
                    "event {} points at board/node that was not shipped in the fork",
                    src_event_id
                ),
            });
            continue;
        }
        if event_proto
            .canary
            .as_ref()
            .is_some_and(|canary| !canary_target_available(canary, &shipped_boards, &maps))
        {
            skipped.push(SkippedItem {
                kind: SkippedKind::Other,
                source_id: src_event_id.clone(),
                reason: format!(
                    "canary target for event {} pointed at a board/node that was not shipped and was cleared",
                    src_event_id
                ),
            });
            event_proto.canary = None;
        }
        drop_unavailable_variants(
            &mut event_proto,
            &src_event_id,
            &shipped_boards,
            &maps,
            &mut skipped,
        );

        if let Some(v) = event_proto.board_version.as_ref() {
            pointed_board_versions
                .insert((event_proto.board_id.clone(), (v.major, v.minor, v.patch)));
        }
        if let Some(canary) = event_proto.canary.as_ref()
            && let Some(v) = canary.board_version.as_ref()
        {
            pointed_board_versions.insert((canary.board_id.clone(), (v.major, v.minor, v.patch)));
        }
        for variant in &event_proto.variants {
            if let Some(v) = variant.board_version.as_ref() {
                pointed_board_versions
                    .insert((variant.board_id.clone(), (v.major, v.minor, v.patch)));
            }
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
            event_proto.config = rewritten.bytes;
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

        // `remap_event` translates `default_page_id` unconditionally, and
        // nothing downstream validates it. Drop the pointer when the page
        // did not ship — whether the fork policy excluded it or its source
        // file was unreadable — so the event doesn't open a page that
        // exists nowhere.
        if let Some(default_page) = event_proto.default_page_id.as_deref()
            && !shipped_pages.contains(default_page)
        {
            event_proto.default_page_id = None;
        }
        for variant in event_proto.variants.iter_mut() {
            if let Some(page) = variant.default_page_id.as_deref()
                && !shipped_pages.contains(page)
            {
                variant.default_page_id = None;
            }
        }

        remap_event(&mut event_proto, &maps);
        let new_event_id = event_proto.id.clone();
        let dst_event_path = dst_events_dir.child(format!("{}.event", new_event_id));
        compress_to_file(dst_meta_store.clone(), dst_event_path, &event_proto)
            .await
            .map_err(|e| ApiError::internal_error(anyhow!("write event: {e}")))?;
        rewritten_events.insert(
            src_event_id,
            flow_like::flow::event::Event::from_proto(event_proto),
        );
    }

    // ---- 4d. Copy versioned boards pointed to by events / canaries ---
    // For each (src_board_id, version) referenced above, load the
    // archived board, remap it (sharing the same `maps` so node/pin ids
    // line up with the live board), and save under the destination
    // versions tree. Version tuple stays the same — we don't re-bump.
    for (src_board_id, version) in &pointed_board_versions {
        if is_reserved_etag_dispatch_version_tuple(*version) {
            skipped.push(SkippedItem {
                kind: SkippedKind::Other,
                source_id: src_board_id.clone(),
                reason: "the version reserved for ETag-bound Latest execution was not copied"
                    .to_string(),
            });
            continue;
        }
        let dst_board_id = maps.translate_board(src_board_id);
        let src_path = src_prefix
            .child("versions")
            .child(src_board_id.clone())
            .child(format!("{}_{}_{}.board", version.0, version.1, version.2));
        let board_proto: proto::Board = match from_compressed::<proto::Board>(
            src_meta_store.clone(),
            src_path,
        )
        .await
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
                skipped.push(SkippedItem {
                        kind: SkippedKind::Other,
                        source_id: src_board_id.clone(),
                        reason: format!(
                            "pinned board version {}.{}.{} could not be read, so events pinned to it have no board to run: {err}",
                            version.0, version.1, version.2
                        ),
                    });
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
    let shipped_widgets = if policy.widgets {
        fork_widgets(
            &src_meta_store,
            &dst_meta_store,
            &src_prefix,
            &dst_prefix,
            &maps,
            &mut skipped,
        )
        .await?
    } else {
        for src_widget_id in maps.widgets.keys() {
            skipped.push(SkippedItem {
                kind: SkippedKind::Policy,
                source_id: src_widget_id.clone(),
                reason: "widgets are excluded by the source app's fork policy".to_string(),
            });
        }
        if !maps.widgets.is_empty() {
            warnings.push(
                "Widgets were not copied. Nodes that instantiate a widget by id will fail at run time."
                    .to_string(),
            );
        }
        HashSet::new()
    };

    // ---- 4c. Templates: load proto::Board, run remap, save -----------
    // Templates *are* boards on disk (`{template_id}.template` is a
    // serialized proto::Board), so we run the same `remap_board` pass
    // we use for live boards. The template id rewrite is layered on top.
    let shipped_templates = if policy.templates {
        fork_templates(
            &src_meta_store,
            &dst_meta_store,
            &src_prefix,
            &dst_prefix,
            &mut maps,
            &mut skipped,
        )
        .await?
    } else {
        for src_template_id in maps.templates.keys() {
            skipped.push(SkippedItem {
                kind: SkippedKind::Policy,
                source_id: src_template_id.clone(),
                reason: "templates are excluded by the source app's fork policy".to_string(),
            });
        }
        HashSet::new()
    };

    // ---- 5/6. Copy content-store files --------------------------------
    // Mirror metadata/ + upload/ + storage/ from src content → dst
    // content. Both stores live under `apps/{id}/` so the prefix
    // translation is just the id substitution. `storage/` carries the
    // project LanceDB (`storage/db/**`) — tables, graph overlays and
    // saved queries all ride along as plain objects.
    //
    // Every mirror below is deliberately scoped to `apps/{id}` (plus the
    // app's own metadata media). User-scoped databases live at
    // `users/{sub}/apps/{id}/db` and are **never** forked: they are
    // per-user working data, not part of the app, and the destination
    // starts them empty.
    copy_metadata_with_translation(
        &src_content_store,
        &dst_content_store,
        &src_prefix.child("metadata"),
        &dst_prefix.child("metadata"),
        &maps,
        &shipped_widgets,
        &shipped_templates,
    )
    .await?;

    let mut copy_tally = CopyTally::default();
    if policy.files {
        copy_tally.add(
            copy_object_prefix(
                &src_content_store,
                &dst_content_store,
                &src_prefix.child("upload"),
                &dst_prefix.child("upload"),
                "upload storage",
                None,
            )
            .await?,
        );
    } else {
        skipped.push(SkippedItem {
            kind: SkippedKind::Policy,
            source_id: "upload/".to_string(),
            reason: "uploaded files are excluded by the source app's fork policy".to_string(),
        });
        warnings.push(
            "Uploaded files were not copied. Flow paths pointing at them will not resolve in the fork."
                .to_string(),
        );
    }

    // `storage/` is two things: the project LanceDB under `storage/db/**`
    // and flow-written scratch under `storage/{node_id}/`. Only the former
    // is policy-gated, so the skip predicate is per-object rather than a
    // whole-prefix branch.
    let storage_skip = policy::storage_skip(&policy);
    copy_tally.add(
        copy_object_prefix(
            &src_content_store,
            &dst_content_store,
            &src_prefix.child("storage"),
            &dst_prefix.child("storage"),
            "app storage",
            storage_skip.as_deref(),
        )
        .await?,
    );

    // Metadata media is never policy-gated: `Meta.icon` / `thumbnail` /
    // `preview_media` carry ids verbatim into the destination rows, so
    // dropping the bytes would leave broken images everywhere.
    copy_tally.add(
        copy_object_prefix(
            &src_content_store,
            &dst_content_store,
            &Path::from("media")
                .child("apps")
                .child(src_app_id.to_string()),
            &Path::from("media").child("apps").child(new_app_id.clone()),
            "app metadata media",
            None,
        )
        .await?,
    );

    match policy.databases {
        ForkDatabaseMode::WithData => {}
        ForkDatabaseMode::None => skipped.push(SkippedItem {
            kind: SkippedKind::Policy,
            source_id: "storage/db".to_string(),
            reason: "the project database is excluded by the source app's fork policy".to_string(),
        }),
        ForkDatabaseMode::SchemaOnly => {
            let created =
                db_schema::copy_project_db_schemas(state, src_app_id, &new_app_id).await?;
            skipped.push(SkippedItem {
                kind: SkippedKind::Policy,
                source_id: "storage/db".to_string(),
                reason: format!(
                    "{} table(s) were recreated empty — the source app's fork policy excludes database rows",
                    created.len()
                ),
            });
            if !created.is_empty() {
                warnings.push(format!(
                    "{} database table(s) were recreated empty. Indices were not copied — rebuild them in Data Studio.",
                    created.len()
                ));
            }
        }
    }

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
        .filter(|b| shipped_boards.contains(*b))
        .map(|b| maps.translate_board(b))
        .collect();
    src_app_proto.events = dedupe_translated(
        src_app_proto
            .events
            .iter()
            .cloned()
            .chain(src_event_rows.iter().map(|r| r.id.clone()))
            .filter(|id| rewritten_events.contains_key(id)),
        |id| maps.translate_event(&id),
    );
    src_app_proto.page_ids = dedupe_translated(
        src_app_proto
            .page_ids
            .iter()
            .cloned()
            .chain(src_page_rows.iter().map(|r| r.id.clone()))
            .filter(|id| shipped_pages.contains(id)),
        |id| maps.translate_page(&id),
    );
    src_app_proto.widget_ids = dedupe_translated(
        src_app_proto
            .widget_ids
            .iter()
            .cloned()
            .chain(src_widget_rows.iter().map(|r| r.id.clone()))
            .filter(|id| shipped_widgets.contains(id)),
        |id| translate_in_map(&maps.widgets, &id),
    );
    src_app_proto.templates = dedupe_translated(
        src_app_proto
            .templates
            .iter()
            .cloned()
            .chain(src_template_rows.iter().map(|r| r.id.clone()))
            .filter(|id| shipped_templates.contains(id)),
        |id| translate_in_map(&maps.templates, &id),
    );
    // Routes are authoritatively stored on the Event row's `route`
    // column — the manifest's `route_mappings` map is a denormalized
    // copy that endpoints don't always keep in sync. Rebuild it from
    // the source events that actually carry routes; that way the
    // destination's manifest reflects current state regardless of
    // whether the source manifest had drifted.
    let mut new_routes = HashMap::new();
    for e in rewritten_events.values() {
        if let Some(path) = e.route.as_ref() {
            new_routes.insert(path.clone(), e.id.clone());
        }
    }
    for (path, event_id) in src_app_proto.route_mappings.iter() {
        if rewritten_events.contains_key(event_id) {
            new_routes
                .entry(path.clone())
                .or_insert_with(|| maps.translate_event(event_id));
        }
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
    // Excluding roles never means "no roles": an app is unusable without an
    // owner role (Membership.role_id is NOT NULL) and a NULL default role
    // breaks every join / invite / purchase path. The destination gets a
    // freshly minted Owner / Admin / User set instead, matching a
    // newly created app.
    let roles_to_copy: Vec<role::Model> = if policy.roles {
        src_role_rows
    } else {
        for r in &src_role_rows {
            skipped.push(SkippedItem {
                kind: SkippedKind::Policy,
                source_id: r.id.clone(),
                reason: "roles are excluded by the source app's fork policy".to_string(),
            });
        }
        if !src_role_rows.is_empty() {
            warnings.push(
                "Roles were not copied — the fork starts with fresh Owner, Admin and User roles. Nodes that reference a role by ID will not resolve (references by name still work)."
                    .to_string(),
            );
        }
        Vec::new()
    };

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
    let shipped_event_id_map: HashMap<String, String> = maps
        .events
        .iter()
        .filter(|(src_id, _)| rewritten_events.contains_key(*src_id))
        .map(|(src_id, dst_id)| (src_id.clone(), dst_id.clone()))
        .collect();
    let (sinks_to_insert, sink_skips) = prepare_dst_sinks(
        &src_sink_rows,
        &shipped_event_id_map,
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
    let events_to_insert: Vec<flow_like::flow::event::Event> =
        rewritten_events.values().cloned().collect();
    let shipped_pages_for_txn = shipped_pages.clone();
    let shipped_widgets_for_txn = shipped_widgets.clone();
    let shipped_templates_for_txn = shipped_templates.clone();

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
                    // A fork of an agent is still an agent.
                    app_type: Set(src_app_row.app_type.clone()),
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
                    // A fork does not inherit the source's fork policy — the
                    // new owner opts in and picks their own.
                    fork_policy: Set(None),
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
                        group_id: Set(None),
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
                // The role map is pre-allocated for every source role
                // regardless of the fork policy, so a mapped id is only
                // safe to point `App.ownerRoleId` / `defaultRoleId` at
                // once the row behind it exists.
                let mut inserted_role_ids: HashSet<String> = HashSet::new();
                for r in &roles_to_copy {
                    let new_role_id = maps_arc.roles.get(&r.id).cloned().unwrap_or_else(create_id);
                    let new_role = role::ActiveModel {
                        id: Set(new_role_id.clone()),
                        name: Set(r.name.clone()),
                        description: Set(r.description.clone()),
                        permissions: Set(r.permissions),
                        app_id: Set(Some(new_app_id_db.clone())),
                        attributes: Set(r.attributes.clone()),
                        created_at: Set(now),
                        updated_at: Set(now),
                    };
                    new_role.insert(txn).await?;
                    inserted_role_ids.insert(new_role_id);
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
                    .filter(|id| inserted_role_ids.contains(id))
                {
                    Some(id) => id,
                    None => {
                        // Source had no owner role recorded, its pointer
                        // was stale, or the fork policy excluded roles.
                        // Invent one so the destination is at least valid;
                        // caller becomes the owner regardless.
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
                // A NULL default role is a live footgun: it hard-fails
                // every join-request, invite, purchase and role-delete
                // path. When the source pointer doesn't resolve to an
                // inserted row, mint the same Admin + User pair a newly
                // created app gets and make User the default.
                let dst_default_role_id = match src_default_role_id
                    .as_deref()
                    .and_then(|src_id| maps_arc.roles.get(src_id).cloned())
                    .filter(|id| inserted_role_ids.contains(id))
                {
                    Some(id) => Some(id),
                    None => {
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

                        let mut user_permission = RolePermissions::ReadTemplates;
                        user_permission.insert(RolePermissions::ExecuteEvents);
                        user_permission.insert(RolePermissions::ListEvents);
                        let user_role_id = create_id();
                        let user_role = role::ActiveModel {
                            id: Set(user_role_id.clone()),
                            name: Set("User".to_string()),
                            description: Set(Some("User role".to_string())),
                            permissions: Set(user_permission.bits()),
                            app_id: Set(Some(new_app_id_db.clone())),
                            attributes: NotSet,
                            created_at: Set(now),
                            updated_at: Set(now),
                        };
                        user_role.insert(txn).await?;
                        Some(user_role_id)
                    }
                };

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

                for e in &events_to_insert {
                    let mut new_event = event_to_db_model(&new_app_id_db, e);
                    new_event.created_at = Set(now);
                    new_event.updated_at = Set(now);
                    new_event.insert(txn).await?;
                }

                for p in src_page_rows
                    .iter()
                    .filter(|p| shipped_pages_for_txn.contains(&p.id))
                {
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

                for w in src_widget_rows
                    .iter()
                    .filter(|w| shipped_widgets_for_txn.contains(&w.id))
                {
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
                    let Some(src_widget_id) = m.widget_id.as_ref() else {
                        continue;
                    };
                    if !shipped_widgets_for_txn.contains(src_widget_id) {
                        continue;
                    }
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
                        group_id: Set(None),
                        created_at: Set(now),
                        updated_at: Set(now),
                    };
                    new_meta.insert(txn).await?;
                }

                for t in src_template_rows
                    .iter()
                    .filter(|t| shipped_templates_for_txn.contains(&t.id))
                {
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
                    let Some(src_template_id) = m.template_id.as_ref() else {
                        continue;
                    };
                    if !shipped_templates_for_txn.contains(src_template_id) {
                        continue;
                    }
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
                        group_id: Set(None),
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
    Ok((
        new_app_id,
        ForkReport {
            id_map: maps,
            skipped,
            warnings,
            bytes_copied: copy_tally.bytes,
            objects_copied: copy_tally.objects,
        },
    ))
}

/// Offline → online uploads come from the desktop content layout, where
/// app metadata images live under `apps/{id}/media`. The API serves and
/// presigns those same images from `media/apps/{id}`. Finalization moves
/// the uploaded files into the server layout, then drops the transient
/// desktop-layout copy so app content accounting does not double-count it.
pub async fn materialize_uploaded_app_media(
    state: &AppState,
    app_id: &str,
) -> Result<(), ApiError> {
    let credentials = state.master_credentials().await?;
    let content_store = credentials.to_store(false).await?.as_generic();
    let src_media_dir = Path::from("apps").child(app_id.to_string()).child("media");
    let dst_media_dir = Path::from("media").child("apps").child(app_id.to_string());

    // NOT a fork: this is offline → online upload finalization, and the
    // source is deleted immediately below. A skip predicate here would
    // destroy the objects it declined to copy — always pass `None`.
    copy_object_prefix(
        &content_store,
        &content_store,
        &src_media_dir,
        &dst_media_dir,
        "uploaded app metadata media",
        None,
    )
    .await?;
    delete_object_prefix(
        &content_store,
        &src_media_dir,
        "uploaded app metadata media",
    )
    .await?;
    Ok(())
}

/// Offline → online uses normal metadata upsert endpoints, and those
/// endpoints intentionally preserve existing media fields. During a fork
/// the destination metadata rows are fresh defaults, so restore icon,
/// thumbnail and preview media from the uploaded local metadata files.
pub async fn sync_uploaded_metadata_media_to_db(
    state: &AppState,
    app_id: &str,
) -> Result<(), ApiError> {
    let credentials = state.master_credentials().await?;
    let content_store = credentials.to_store(false).await?.as_generic();
    let metadata_dir = Path::from("apps")
        .child(app_id.to_string())
        .child("metadata");
    let metadata_dir_str = metadata_dir.as_ref().to_string();

    let mut listing = content_store.list(Some(&metadata_dir));
    let now = chrono::Utc::now().naive_utc();
    while let Some(item) = listing
        .try_next()
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("list uploaded metadata: {e}")))?
    {
        let path_str = item.location.as_ref().to_string();
        let Some(relative_path) = relative_to_prefix(&path_str, &metadata_dir_str) else {
            continue;
        };
        let Some(target) = UploadedMetadataTarget::from_relative_path(&relative_path) else {
            continue;
        };

        let uploaded: proto::Metadata =
            match from_compressed(content_store.clone(), item.location.clone()).await {
                Ok(metadata) => metadata,
                Err(err) => {
                    tracing::warn!(
                        app_id = %app_id,
                        path = %path_str,
                        "skip uploaded metadata media sync: {}",
                        err,
                    );
                    continue;
                }
            };

        update_meta_media_fields(state, app_id, target, uploaded, now).await?;
    }

    Ok(())
}

// ---- helpers ----------------------------------------------------------

fn remap_board(mut board: proto::Board, maps: &mut ForkIdMap) -> proto::Board {
    // Host receipts belong to the source board's persistence boundary and must never be copied
    // into a fork where their identities and replay claims are invalid.
    board.internal_refs.clear();
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
    for node in nodes.values() {
        maps.nodes.entry(node.id.clone()).or_insert_with(create_id);
        register_pin_ids(&node.pins, maps);
    }
}

fn register_pin_ids(pins: &HashMap<String, proto::Pin>, maps: &mut ForkIdMap) {
    for pin in pins.values() {
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
/// known source id, plus any page-scoped element reference whose page
/// head is a known source page. Returns whether anything changed so
/// callers can skip a re-encode when the payload is untouched.
fn translate_ids_in_json(value: &mut flow_like_types::Value, maps: &ForkIdMap) -> bool {
    match value {
        flow_like_types::Value::String(s) => {
            let translated = lookup_id(s, maps).or_else(|| translate_element_ref(s, maps));
            if let Some(translated) = translated {
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

/// Whole-string translation of any id a pin default may name.
///
/// `widgets` is in this chain because the `widget_selector` pin of
/// `a2ui_instantiate_widget` stores the bare project widget id (see
/// `WidgetVariable` / `widget-select.tsx`, which commits
/// `selector: widgetId`), and `fork_widgets` gives every copied widget
/// a fresh id — so without this the forked node resolves against the
/// source app's widget and fails with "Widget '…' not found". Package
/// widget selectors are immune by construction: they are encoded
/// `pkg:{package_id}/{widget_id}`, never a key of `maps.widgets`, and
/// package ids are global and must never be rewritten. Legacy
/// name-based selectors are likewise untouched.
///
/// `roles` is here because the `role` pin of the project-user nodes
/// accepts "Role ID or exact role name" — an id needs translating, a
/// name is not a map key and passes through.
///
/// Deliberately absent: `templates` (no board artifact stores a
/// template id) and `variables` (variable ids are preserved verbatim
/// by `remap_board`, so `var_ref` defaults must keep resolving against
/// the unchanged `board.variables` keys).
fn lookup_id(src: &str, maps: &ForkIdMap) -> Option<String> {
    maps.nodes
        .get(src)
        .or_else(|| maps.layers.get(src))
        .or_else(|| maps.events.get(src))
        .or_else(|| maps.pages.get(src))
        .or_else(|| maps.pins.get(src))
        .or_else(|| maps.boards.get(src))
        .or_else(|| maps.widgets.get(src))
        .or_else(|| maps.roles.get(src))
        .cloned()
}

/// UI element references are composite: the element picker stores
/// `"{page_id}/{component_id}"` in the `element_ref` pin default (see
/// `ElementSelect`), and the runtime keys its `_elements` payload the
/// same way — the prerun manifest lists the refs a board reads, and
/// `ExecutionContext::read_element` resolves them by exact key first,
/// then by `/{component_id}` suffix on the shipped map.
///
/// Component ids are page-scoped and survive a fork unchanged, but the
/// page head does not, so `lookup_id` never matches the composite
/// string as a whole. Without this pass every `Get Element` /
/// `Set Element …` node in a forked app keeps pointing at the source
/// app's page and silently resolves to "element not found".
fn translate_element_ref(src: &str, maps: &ForkIdMap) -> Option<String> {
    let (page_id, component_id) = src.split_once('/')?;
    if component_id.is_empty() {
        return None;
    }
    let new_page_id = maps.pages.get(page_id)?;
    Some(format!("{}/{}", new_page_id, component_id))
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
    for variant in event.variants.iter_mut() {
        variant.board_id = maps.translate_board(&variant.board_id);
        variant.node_id = maps.translate_node(&variant.node_id);
        if let Some(page) = variant.default_page_id.as_ref() {
            variant.default_page_id = Some(maps.translate_page(page));
        }
    }
    for input in event.inputs.iter_mut() {
        if let Some(new_pin) = maps.pins.get(&input.id) {
            input.id = new_pin.clone();
        }
    }

    strip_event_secrets(event);
}

/// Clears `default_value` on every secret-marked variable inside an event
/// proto, including the canary's and every variant's variables. The event's
/// `config` bytes are intentionally NOT touched here — token sites (HTTP
/// auth_token, PAT, OAuth) are replaced in Phase 4 with caller-supplied
/// values.
fn strip_event_secrets(event: &mut proto::Event) {
    strip_secret_values(&mut event.variables);
    if let Some(canary) = event.canary.as_mut() {
        strip_secret_values(&mut canary.variables);
    }
    for variant in event.variants.iter_mut() {
        strip_secret_values(&mut variant.variables);
    }
}

fn strip_secret_values(variables: &mut HashMap<String, proto::Variable>) {
    for var in variables.values_mut() {
        if var.secret {
            var.default_value.clear();
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

/// Append a `/`-separated *relative* path below `prefix`, one segment at
/// a time.
///
/// `Path::child` treats its argument as a single `PathPart` and
/// percent-encodes the delimiter, so `prefix.child("db/x.lance/data/y")`
/// yields `prefix/db%2Fx.lance%2Fdata%2Fy` — one flat key instead of a
/// nested path. Every content mirror below has to fold per segment or
/// the destination silently ends up with garbage keys (this is what
/// used to drop the entire project LanceDB under `storage/db/**` on
/// online → online forks).
fn join_relative(prefix: &Path, relative: &str) -> Path {
    relative
        .split('/')
        .filter(|segment| !segment.is_empty())
        .fold(prefix.clone(), |acc, segment| acc.child(segment))
}

/// Bytes + objects a single prefix mirror moved. Summed into
/// [`ForkReport::bytes_copied`] / [`ForkReport::objects_copied`].
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct CopyTally {
    pub objects: u64,
    pub bytes: u64,
}

impl CopyTally {
    fn add(&mut self, other: CopyTally) {
        self.objects = self.objects.saturating_add(other.objects);
        self.bytes = self.bytes.saturating_add(other.bytes);
    }
}

/// Mirrors `src_prefix` onto `dst_prefix`.
///
/// `skip_relative` is evaluated against the source-relative suffix (e.g.
/// `db/foo.lance/data/0.lance` under `apps/{id}/storage`) and decides
/// *before* an object is scheduled, so a policy exclusion is never
/// confused with a copy failure and excluded objects never enter the
/// concurrency fan-out.
async fn copy_object_prefix(
    src_store: &Arc<dyn flow_like_storage::object_store::ObjectStore>,
    dst_store: &Arc<dyn flow_like_storage::object_store::ObjectStore>,
    src_prefix: &Path,
    dst_prefix: &Path,
    label: &str,
    skip_relative: Option<&(dyn Fn(&str) -> bool + Send + Sync)>,
) -> Result<CopyTally, ApiError> {
    use futures::StreamExt;

    let mut listing = src_store.list(Some(src_prefix));
    let mut entries: Vec<(Path, Path)> = Vec::new();
    let mut tally = CopyTally::default();
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
        if skip_relative.is_some_and(|skip| skip(suffix)) {
            continue;
        }
        let dst_path = join_relative(dst_prefix, suffix);
        tally.bytes = tally.bytes.saturating_add(item.size);
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
        tally.objects = tally.objects.saturating_add(1);
    }
    Ok(tally)
}

async fn delete_object_prefix(
    store: &Arc<dyn flow_like_storage::object_store::ObjectStore>,
    prefix: &Path,
    label: &str,
) -> Result<(), ApiError> {
    use futures::StreamExt;

    let mut listing = store.list(Some(prefix));
    let mut paths: Vec<Path> = Vec::new();
    while let Some(item) = listing
        .try_next()
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("list {label}: {e}")))?
    {
        paths.push(item.location);
    }

    let results: Vec<Result<(), ApiError>> =
        futures::stream::iter(paths.into_iter().map(|path| async move {
            store
                .delete(&path)
                .await
                .map_err(|e| ApiError::internal_error(anyhow!("delete {label}: {e}")))
        }))
        .buffer_unordered(COPY_CONCURRENCY)
        .collect()
        .await;
    for result in results {
        result?;
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum UploadedMetadataTarget {
    App { lang: String },
    Widget { id: String, lang: String },
    Template { id: String, lang: String },
}

impl UploadedMetadataTarget {
    fn from_relative_path(relative_path: &str) -> Option<Self> {
        let segments: Vec<&str> = relative_path
            .split('/')
            .filter(|segment| !segment.is_empty())
            .collect();
        match segments.as_slice() {
            [file] => Some(Self::App {
                lang: lang_from_meta_file(file)?,
            }),
            ["widgets", id, file] => Some(Self::Widget {
                id: (*id).to_string(),
                lang: lang_from_meta_file(file)?,
            }),
            ["templates", id, file] => Some(Self::Template {
                id: (*id).to_string(),
                lang: lang_from_meta_file(file)?,
            }),
            _ => None,
        }
    }
}

fn lang_from_meta_file(file_name: &str) -> Option<String> {
    file_name
        .strip_suffix(".meta")
        .filter(|lang| !lang.is_empty())
        .map(ToString::to_string)
}

async fn update_meta_media_fields(
    state: &AppState,
    app_id: &str,
    target: UploadedMetadataTarget,
    uploaded: proto::Metadata,
    now: chrono::NaiveDateTime,
) -> Result<(), ApiError> {
    let row = match &target {
        UploadedMetadataTarget::App { lang } => {
            meta::Entity::find()
                .filter(meta::Column::AppId.eq(app_id))
                .filter(meta::Column::Lang.eq(lang))
                .one(&state.db)
                .await?
        }
        UploadedMetadataTarget::Widget { id, lang } => {
            meta::Entity::find()
                .filter(meta::Column::WidgetId.eq(id))
                .filter(meta::Column::Lang.eq(lang))
                .one(&state.db)
                .await?
        }
        UploadedMetadataTarget::Template { id, lang } => {
            meta::Entity::find()
                .filter(meta::Column::TemplateId.eq(id))
                .filter(meta::Column::Lang.eq(lang))
                .one(&state.db)
                .await?
        }
    };

    let Some(row) = row else {
        tracing::warn!(
            app_id = %app_id,
            target = ?target,
            "skip uploaded metadata media sync: destination metadata row not found",
        );
        return Ok(());
    };

    let preview_media = if uploaded.preview_media.is_empty() {
        None
    } else {
        Some(uploaded.preview_media)
    };
    let mut active = row.into_active_model();
    active.icon = Set(uploaded.icon);
    active.thumbnail = Set(uploaded.thumbnail);
    active.preview_media = Set(preview_media);
    active.updated_at = Set(now);
    active.update(&state.db).await?;
    Ok(())
}

fn relative_to_prefix(path: &str, prefix: &str) -> Option<String> {
    let suffix = path.strip_prefix(prefix)?;
    if !suffix.is_empty() && !suffix.starts_with('/') {
        return None;
    }
    Some(suffix.trim_start_matches('/').to_string())
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

/// Everything a fork must rewrite inside one page payload, and the
/// payloads it could not rewrite.
///
/// A page that reaches the destination with even one un-rewritten
/// reference keeps firing the *source* app's nodes, so a failure here
/// is reported rather than logged — see [`SkippedKind::Other`].
#[derive(Debug, Default)]
struct RemapIssues {
    /// `component id: reason` for every JSON payload that could not be
    /// parsed or re-encoded, and therefore still carries source ids.
    unrewritten: Vec<String>,
}

impl RemapIssues {
    fn is_empty(&self) -> bool {
        self.unrewritten.is_empty()
    }

    /// One line, not a transcript. The whole report is rendered in a dialog
    /// and handed verbatim to the agent's `fork_app` tool, so a badly damaged
    /// app must not be able to turn one page into kilobytes of prose.
    fn reason(&self, subject: &str) -> String {
        const NAMED: usize = 5;
        let listed = self
            .unrewritten
            .iter()
            .take(NAMED)
            .cloned()
            .collect::<Vec<_>>()
            .join("; ");
        let remainder = self.unrewritten.len().saturating_sub(NAMED);
        let tail = if remainder > 0 {
            format!(" (+{remainder} more)")
        } else {
            String::new()
        };
        format!(
            "{subject} kept {} reference payload(s) the fork could not rewrite, so they still point at the source app: {listed}{tail}",
            self.unrewritten.len()
        )
    }
}

/// Apply the fork's id translations to a page in-place.
///
/// Which references exist inside a page payload — the board, the three
/// behaviour hooks (**node** ids despite being named `*_event_id`),
/// every widget instance and its action bindings, the embedded widget
/// definitions, and the opaque JSON under all of them — is
/// [`flow_like::a2ui::page_remap`]'s inventory, shared with template
/// instantiation. This function supplies only the fork's policy: the
/// page's new id, and what each source id becomes.
///
/// Without it a forked app's pages still load, but every "on click run
/// this workflow / navigate to this page / show this widget" hook
/// silently points at the source app's ids.
fn remap_page(page: &mut proto::Page, new_page_id: &str, maps: &ForkIdMap) -> RemapIssues {
    page.id = new_page_id.to_string();
    let mut by_field = fork_field_translator(maps);
    let mut by_literal = fork_literal_translator(maps);
    let mut translators = page_remap::IdTranslators {
        by_field: &mut by_field,
        by_literal: &mut by_literal,
    };
    RemapIssues {
        unrewritten: page_remap::remap_page_refs(page, &mut translators),
    }
}

/// Resolve a reference the a2ui walker found under a recognized field
/// name. Translation stays opt-in per value: a name only resolves when
/// the embedded string is actually a key of the corresponding map, so a
/// user-authored `nodeId` in unrelated game state is left alone, and a
/// value already on the destination's id space is a no-op.
fn fork_field_translator(maps: &ForkIdMap) -> impl FnMut(IdRef, &str) -> Option<String> + '_ {
    move |kind, id| match kind {
        IdRef::Node => maps.nodes.get(id).cloned(),
        IdRef::Board => maps.boards.get(id).cloned(),
        IdRef::Page => maps.pages.get(id).cloned(),
        IdRef::Widget => maps.widgets.get(id).cloned(),
        IdRef::Event => maps.events.get(id).cloned(),
        IdRef::App => {
            (id == maps.source_app_id && !maps.source_app_id.is_empty() && !maps.app_id.is_empty())
                .then(|| maps.app_id.clone())
        }
    }
}

/// Resolve a reference that arrived without a field name — a widget
/// customization value, an exposed prop's default. `lookup_id` is the
/// same whole-string pass pin defaults take, so the two agree on what
/// counts as an id, and composite element references
/// (`{page_id}/{component_id}`) follow the page they name.
fn fork_literal_translator(maps: &ForkIdMap) -> impl FnMut(&str) -> Option<String> + '_ {
    move |id| lookup_id(id, maps).or_else(|| translate_element_ref(id, maps))
}

/// Run the shared widget pass over a JSON-serialized `.widget` document with
/// this fork's translators.
fn remap_widget_json(widget: &mut flow_like_types::Value, maps: &ForkIdMap) -> Vec<String> {
    let mut by_field = fork_field_translator(maps);
    let mut by_literal = fork_literal_translator(maps);
    let mut translators = page_remap::IdTranslators {
        by_field: &mut by_field,
        by_literal: &mut by_literal,
    };
    page_remap::remap_widget_json(widget, &mut translators)
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
///
/// A page that cannot travel is reported, not just logged: the
/// destination is missing an interface its board still expects, and
/// only the caller can tell the user.
async fn fork_pages_db_driven(
    src_store: &Arc<dyn flow_like_storage::object_store::ObjectStore>,
    dst_store: &Arc<dyn flow_like_storage::object_store::ObjectStore>,
    src_prefix: &Path,
    dst_prefix: &Path,
    src_page_rows: &[page::Model],
    maps: &ForkIdMap,
    shipped_boards: &HashSet<String>,
    skipped: &mut Vec<SkippedItem>,
) -> Result<HashSet<String>, ApiError> {
    let mut shipped_pages = HashSet::new();
    for row in src_page_rows {
        let src_page_id = row.id.clone();
        let new_page_id = maps.translate_page(&src_page_id);
        let Some(src_board_id) = row.board_id.as_deref() else {
            tracing::warn!("skip page {} during fork: row has no board_id", src_page_id);
            skipped.push(SkippedItem {
                kind: SkippedKind::Other,
                source_id: src_page_id,
                reason: "page row names no board, so it has no place in the destination"
                    .to_string(),
            });
            continue;
        };
        let Some(new_board_id) = maps
            .boards
            .get(src_board_id)
            .filter(|_| shipped_boards.contains(src_board_id))
            .cloned()
        else {
            tracing::warn!(
                "skip page {} during fork: row points at board {} which was not shipped",
                src_page_id,
                src_board_id
            );
            skipped.push(SkippedItem {
                kind: SkippedKind::Other,
                source_id: src_page_id,
                reason: format!(
                    "page points at board {} which was not shipped in the fork",
                    src_board_id
                ),
            });
            continue;
        };

        let mut page_proto = match read_source_page(
            src_store,
            src_prefix,
            Some(src_board_id),
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
                skipped.push(SkippedItem {
                    kind: SkippedKind::Other,
                    source_id: src_page_id,
                    reason: "page has no readable source file at either the app-level or the board-scoped path".to_string(),
                });
                continue;
            }
        };

        let issues = remap_page(&mut page_proto, &new_page_id, maps);
        if !issues.is_empty() {
            skipped.push(SkippedItem {
                kind: SkippedKind::Other,
                source_id: src_page_id.clone(),
                reason: issues.reason("page"),
            });
        }
        write_destination_page(
            dst_store,
            dst_prefix,
            Some(new_board_id.as_str()),
            &page_proto,
        )
        .await?;
        shipped_pages.insert(src_page_id);
    }
    Ok(shipped_pages)
}

fn translate_in_map(map: &HashMap<String, String>, src: &str) -> String {
    map.get(src).cloned().unwrap_or_else(|| src.to_string())
}

fn overlay_board_page_ids_from_rows(
    board: &mut proto::Board,
    src_board_id: &str,
    page_rows: &[page::Model],
) {
    let mut seen: HashSet<String> = board.page_ids.iter().cloned().collect();
    for row in page_rows {
        if row.board_id.as_deref() == Some(src_board_id) && seen.insert(row.id.clone()) {
            board.page_ids.push(row.id.clone());
        }
    }
}

fn retain_shipped_board_pages(board: &mut proto::Board, shipped_dst_page_ids: &HashSet<String>) {
    let mut seen = HashSet::new();
    board
        .page_ids
        .retain(|id| shipped_dst_page_ids.contains(id) && seen.insert(id.clone()));
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
/// can contain the same action/reference JSON shape as pages, so those
/// references are remapped before writing.
///
/// Widget metadata is handled by `copy_metadata_with_translation` — this
/// pass only touches the `{widget_id}.widget` JSON files.
async fn fork_widgets(
    src_store: &Arc<dyn flow_like_storage::object_store::ObjectStore>,
    dst_store: &Arc<dyn flow_like_storage::object_store::ObjectStore>,
    src_prefix: &Path,
    dst_prefix: &Path,
    maps: &ForkIdMap,
    skipped: &mut Vec<SkippedItem>,
) -> Result<HashSet<String>, ApiError> {
    let mut shipped_widgets = HashSet::new();
    for (src_widget_id, new_widget_id) in &maps.widgets {
        let src_path = src_prefix.child(format!("{}.widget", src_widget_id));
        let mut widget: flow_like_types::Value =
            match from_compressed_json(src_store.clone(), src_path).await {
                Ok(w) => w,
                Err(err) => {
                    tracing::warn!("skip widget {}: {}", src_widget_id, err);
                    skipped.push(SkippedItem {
                        kind: SkippedKind::Other,
                        source_id: src_widget_id.clone(),
                        reason: format!(
                            "widget definition could not be read from the source app: {err}"
                        ),
                    });
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
        // `actionBindings` shapes that pages do, and its exposed-prop and
        // customization defaults hide ids inside byte arrays, so the whole
        // document goes through the shared widget pass.
        for issue in remap_widget_json(&mut widget, maps) {
            skipped.push(SkippedItem {
                kind: SkippedKind::Other,
                source_id: src_widget_id.clone(),
                reason: format!(
                    "widget kept a reference payload the fork could not rewrite: {issue}"
                ),
            });
        }
        let dst_path = dst_prefix.child(format!("{}.widget", new_widget_id));
        compress_to_file_json(dst_store.clone(), dst_path, &widget)
            .await
            .map_err(|e| ApiError::internal_error(anyhow!("write widget: {e}")))?;
        shipped_widgets.insert(src_widget_id.clone());
    }
    Ok(shipped_widgets)
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
    skipped: &mut Vec<SkippedItem>,
) -> Result<HashSet<String>, ApiError> {
    let mut shipped_templates = HashSet::new();
    let template_pairs: Vec<(String, String)> = maps
        .templates
        .iter()
        .map(|(s, d)| (s.clone(), d.clone()))
        .collect();
    for (src_template_id, new_template_id) in template_pairs {
        let template_page_ids =
            list_template_page_ids(src_store, src_prefix, &src_template_id, skipped).await;
        for src_page_id in &template_page_ids {
            maps.pages
                .entry(src_page_id.clone())
                .or_insert_with(create_id);
        }
        let src_path = src_prefix.child(format!("{}.template", src_template_id));
        let board_proto: proto::Board =
            match from_compressed::<proto::Board>(src_store.clone(), src_path).await {
                Ok(b) => b,
                Err(err) => {
                    tracing::warn!("skip template {}: {}", src_template_id, err);
                    skipped.push(SkippedItem {
                        kind: SkippedKind::Other,
                        source_id: src_template_id.clone(),
                        reason: format!(
                            "template board could not be read from the source app: {err}"
                        ),
                    });
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

        for template_page in read_template_pages(
            src_store,
            src_prefix,
            &src_template_id,
            &template_page_ids,
            maps,
            skipped,
        )
        .await?
        {
            let dst_page_path = dst_prefix
                .child(format!("_template_{}", new_template_id))
                .child(format!("{}.page", template_page.id));
            compress_to_file(dst_store.clone(), dst_page_path, &template_page)
                .await
                .map_err(|e| ApiError::internal_error(anyhow!("write template page: {e}")))?;
        }

        shipped_templates.insert(src_template_id);
    }
    Ok(shipped_templates)
}

/// An object store that is a real filesystem reports a prefix with no
/// objects under it as an error instead of an empty listing, so a
/// template that never had pages looks like a failure. Object stores
/// have no typed variant for this, which is why the desktop's fork
/// applier sniffs the same way.
fn is_missing_prefix(error: &impl std::fmt::Display) -> bool {
    let message = error.to_string();
    message.contains("not found") || message.contains("No such file")
}

/// List the pages a template snapshotted
/// (`_template_{template_id}/{page_id}.page`).
///
/// Separate from reading them because the ids have to reach `maps.pages`
/// *before* the template board is remapped: `remap_board` rewrites
/// `board.page_ids` through that map, and a page the map does not know
/// would keep its source id there while its file landed under a fresh
/// one — a template listing pages that do not exist.
async fn list_template_page_ids(
    src_store: &Arc<dyn flow_like_storage::object_store::ObjectStore>,
    src_prefix: &Path,
    src_template_id: &str,
    skipped: &mut Vec<SkippedItem>,
) -> Vec<String> {
    let template_dir = src_prefix.child(format!("_template_{}", src_template_id));
    let mut listing = src_store.list(Some(&template_dir));
    let mut source_page_ids: Vec<String> = Vec::new();
    loop {
        // A template with no pages has no directory at all, and a filesystem
        // store reports that as an error rather than an empty listing. Losing
        // a template's pages must never cost the caller the whole fork, so a
        // listing failure is reported and the template ships without them.
        match listing.try_next().await {
            Ok(Some(item)) => {
                if let Some(page_id) = item
                    .location
                    .filename()
                    .and_then(|name| name.strip_suffix(".page"))
                {
                    source_page_ids.push(page_id.to_string());
                }
            }
            Ok(None) => break,
            Err(err) if is_missing_prefix(&err) => break,
            Err(err) => {
                tracing::warn!("list template pages for {src_template_id}: {err}");
                skipped.push(SkippedItem {
                    kind: SkippedKind::Other,
                    source_id: src_template_id.to_string(),
                    reason: format!(
                        "template pages could not be listed, so the template ships without its interfaces: {err}"
                    ),
                });
                break;
            }
        }
    }
    source_page_ids
}

/// Read and remap every page a template snapshotted. A template whose
/// board arrives without its interfaces instantiates an empty screen, so
/// these travel with it.
///
/// Every id in `src_page_ids` is expected to be in `maps.pages` already
/// — see [`list_template_page_ids`]. The template's page ids are the
/// *live board's* ids at snapshot time (`Board::create_template` clones
/// the board), so most are there from the live pass; one the fork has
/// never seen is minted by the caller before this runs.
async fn read_template_pages(
    src_store: &Arc<dyn flow_like_storage::object_store::ObjectStore>,
    src_prefix: &Path,
    src_template_id: &str,
    src_page_ids: &[String],
    maps: &ForkIdMap,
    skipped: &mut Vec<SkippedItem>,
) -> Result<Vec<proto::Page>, ApiError> {
    let template_dir = src_prefix.child(format!("_template_{}", src_template_id));
    let mut pages = Vec::with_capacity(src_page_ids.len());
    for src_page_id in src_page_ids {
        let src_path = template_dir.child(format!("{}.page", src_page_id));
        let mut page_proto = match from_compressed::<proto::Page>(src_store.clone(), src_path).await
        {
            Ok(page) => page,
            Err(err) => {
                tracing::warn!(
                    "skip template page {} of template {}: {}",
                    src_page_id,
                    src_template_id,
                    err
                );
                skipped.push(SkippedItem {
                    kind: SkippedKind::Other,
                    source_id: src_page_id.clone(),
                    reason: format!(
                        "page snapshotted into template {src_template_id} could not be read: {err}"
                    ),
                });
                continue;
            }
        };
        let new_page_id = maps.translate_page(src_page_id);
        let issues = remap_page(&mut page_proto, &new_page_id, maps);
        if !issues.is_empty() {
            skipped.push(SkippedItem {
                kind: SkippedKind::Other,
                source_id: src_page_id.clone(),
                reason: issues.reason(&format!("template {src_template_id} page")),
            });
        }
        pages.push(page_proto);
    }
    Ok(pages)
}

/// Copies the `metadata/` subtree from src to dst, rewriting any path
/// segment that names a `widget_id`, `template_id`, or `page_id` so it
/// matches the destination's id space. App-level files (e.g. `metadata/
/// {lang}.meta`) are copied verbatim.
///
/// `shipped_widgets` / `shipped_templates` are the source ids that
/// actually made it into the destination. Metadata for anything else is
/// dropped rather than translated — the fork policy can exclude a whole
/// category, and carrying its metadata would leave rows describing
/// artifacts that do not exist.
async fn copy_metadata_with_translation(
    src_store: &Arc<dyn flow_like_storage::object_store::ObjectStore>,
    dst_store: &Arc<dyn flow_like_storage::object_store::ObjectStore>,
    src_meta_dir: &Path,
    dst_meta_dir: &Path,
    maps: &ForkIdMap,
    shipped_widgets: &HashSet<String>,
    shipped_templates: &HashSet<String>,
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
        if !suffix.is_empty() && !suffix.starts_with('/') {
            continue;
        }
        let suffix = suffix.trim_start_matches('/');
        if suffix.is_empty() {
            continue;
        }

        // Translate the second path segment when the first is a known
        // category. e.g. `widgets/{src_id}/{lang}.meta` becomes
        // `widgets/{dst_id}/{lang}.meta`.
        let translated_suffix = match suffix.split('/').collect::<Vec<_>>().as_slice() {
            ["widgets", id, ..] if !shipped_widgets.contains(*id) => continue,
            ["templates", id, ..] if !shipped_templates.contains(*id) => continue,
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
        let dst_path = join_relative(dst_meta_dir, &dst_suffix);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn page_row(id: &str, board_id: Option<&str>) -> page::Model {
        let now = chrono::Utc::now().naive_utc();
        page::Model {
            id: id.to_string(),
            name: id.to_string(),
            description: None,
            app_id: "src_app".to_string(),
            board_id: board_id.map(str::to_string),
            version: None,
            created_at: now,
            updated_at: now,
        }
    }

    fn proto_version(version: (u32, u32, u32)) -> proto::Version {
        proto::Version {
            major: version.0,
            minor: version.1,
            patch: version.2,
        }
    }

    #[test]
    fn reserved_etag_dispatch_version_is_detected_on_primary_and_canary_selectors() {
        assert!(is_reserved_etag_dispatch_version_tuple(
            ETAG_BOUND_LATEST_VERSION_SENTINEL
        ));
        assert!(!is_reserved_etag_dispatch_version_tuple((1, 2, 3)));

        let mut event = proto::Event {
            board_version: Some(proto_version(ETAG_BOUND_LATEST_VERSION_SENTINEL)),
            canary: Some(proto::Canary {
                board_version: Some(proto_version(ETAG_BOUND_LATEST_VERSION_SENTINEL)),
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut skipped = Vec::new();
        assert!(
            !guard_reserved_event_versions(&mut event, "event-1", &mut skipped),
            "a reserved primary selector causes the whole event to be skipped"
        );
        assert_eq!(skipped.len(), 1);

        event.board_version = Some(proto_version((1, 2, 3)));
        skipped.clear();
        assert!(
            guard_reserved_event_versions(&mut event, "event-1", &mut skipped),
            "a reserved canary selector is cleared while the primary event survives"
        );
        assert!(event.canary.is_none());
        assert_eq!(skipped.len(), 1);

        skipped.clear();
        assert!(guard_reserved_event_versions(
            &mut event,
            "event-1",
            &mut skipped
        ));
        assert!(skipped.is_empty());
    }

    fn proto_variant(name: &str, board_id: &str, node_id: &str) -> proto::EventVariant {
        proto::EventVariant {
            name: name.to_string(),
            board_id: board_id.to_string(),
            node_id: node_id.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn reserved_etag_dispatch_version_drops_only_the_offending_variant() {
        let mut event = proto::Event {
            board_version: Some(proto_version((1, 2, 3))),
            variants: vec![
                proto::EventVariant {
                    board_version: Some(proto_version(ETAG_BOUND_LATEST_VERSION_SENTINEL)),
                    ..proto_variant("pinned", "board-1", "node-1")
                },
                proto_variant("floating", "board-1", "node-1"),
            ],
            ..Default::default()
        };
        let mut skipped = Vec::new();
        assert!(
            guard_reserved_event_versions(&mut event, "event-1", &mut skipped),
            "a reserved variant selector is dropped while the primary event survives"
        );
        assert_eq!(skipped.len(), 1);
        assert_eq!(event.variants.len(), 1);
        assert_eq!(event.variants[0].name, "floating");
    }

    #[test]
    fn unavailable_variants_are_dropped_individually() {
        let mut maps = ForkIdMap::default();
        maps.nodes
            .insert("src_node".to_string(), "dst_node".to_string());
        let shipped_boards = HashSet::from(["shipped".to_string()]);
        let mut event = proto::Event {
            variants: vec![
                proto_variant("kept", "shipped", "src_node"),
                proto_variant("unshipped-board", "missing", "src_node"),
                proto_variant("unshipped-node", "shipped", "missing_node"),
            ],
            ..Default::default()
        };
        let mut skipped = Vec::new();
        drop_unavailable_variants(&mut event, "event-1", &shipped_boards, &maps, &mut skipped);
        assert_eq!(event.variants.len(), 1);
        assert_eq!(event.variants[0].name, "kept");
        assert_eq!(skipped.len(), 2);
    }

    #[test]
    fn remap_event_translates_and_strips_variant_targets() {
        let maps = widget_fork_maps();
        let mut variant = proto_variant("canary", "src_board", "src_node");
        variant.default_page_id = Some("src_page".to_string());
        variant.variables.insert(
            "v1".to_string(),
            proto::Variable {
                secret: true,
                default_value: b"shh".to_vec(),
                ..Default::default()
            },
        );
        let mut event = proto::Event {
            variants: vec![variant],
            ..Default::default()
        };

        remap_event(&mut event, &maps);

        let variant = &event.variants[0];
        assert_eq!(variant.board_id, "dst_board");
        assert_eq!(variant.node_id, "dst_node");
        assert_eq!(variant.default_page_id.as_deref(), Some("dst_page"));
        assert!(variant.variables["v1"].default_value.is_empty());
    }

    #[test]
    fn board_page_ids_are_overlaid_from_db_and_trimmed_to_shipped_pages() {
        let mut board = proto::Board {
            id: "src_board".to_string(),
            page_ids: vec!["stale_page".to_string(), "src_page_1".to_string()],
            ..Default::default()
        };
        let page_rows = vec![
            page_row("src_page_1", Some("src_board")),
            page_row("src_page_2", Some("src_board")),
            page_row("src_page_3", Some("src_board")),
            page_row("other_board_page", Some("other_board")),
            page_row("orphan_page", None),
        ];

        overlay_board_page_ids_from_rows(&mut board, "src_board", &page_rows);
        assert_eq!(
            board.page_ids,
            vec![
                "stale_page".to_string(),
                "src_page_1".to_string(),
                "src_page_2".to_string(),
                "src_page_3".to_string(),
            ]
        );

        let mut maps = ForkIdMap::default();
        maps.pages
            .insert("src_page_1".to_string(), "dst_page_1".to_string());
        maps.pages
            .insert("src_page_2".to_string(), "dst_page_2".to_string());
        maps.pages
            .insert("src_page_3".to_string(), "dst_page_3".to_string());

        board.page_ids = board
            .page_ids
            .iter()
            .map(|id| maps.translate_page(id))
            .collect();
        let shipped_dst_page_ids = ["dst_page_2".to_string(), "dst_page_3".to_string()]
            .into_iter()
            .collect();
        retain_shipped_board_pages(&mut board, &shipped_dst_page_ids);

        assert_eq!(
            board.page_ids,
            vec!["dst_page_2".to_string(), "dst_page_3".to_string()]
        );
    }

    #[test]
    fn json_workflow_refs_translate_node_ids_without_rewriting_normal_event_ids() {
        let mut maps = ForkIdMap::default();
        maps.nodes
            .insert("src_node_a".to_string(), "dst_node_a".to_string());
        maps.nodes
            .insert("src_node_b".to_string(), "dst_node_b".to_string());
        maps.nodes
            .insert("src_node_c".to_string(), "dst_node_c".to_string());
        maps.events
            .insert("src_event".to_string(), "dst_event".to_string());

        let mut value = flow_like_types::json::json!({
            "workflowEvent": {
                "eventId": { "literalString": "src_node_a" }
            },
            "workflow": [
                { "eventId": "src_node_b" },
                { "flowId": "src_node_c" }
            ],
            "eventId": { "literalString": "src_event" }
        });

        let mut translate = fork_field_translator(&maps);
        id_refs::rewrite_json_ids(&mut value, &mut translate);

        assert_eq!(
            value["workflowEvent"]["eventId"]["literalString"],
            "dst_node_a"
        );
        assert_eq!(value["workflow"][0]["eventId"], "dst_node_b");
        assert_eq!(value["workflow"][1]["flowId"], "dst_node_c");
        assert_eq!(value["eventId"]["literalString"], "dst_event");
    }

    #[test]
    fn element_ref_pin_defaults_follow_the_page_into_the_fork() {
        let mut maps = ForkIdMap::default();
        maps.pages
            .insert("src_page".to_string(), "dst_page".to_string());
        maps.pages
            .insert("src_page_2".to_string(), "dst_page_2".to_string());

        let cases = [
            // Picker format: "{page_id}/{component_id}".
            ("src_page/submit-button", "dst_page/submit-button"),
            // Component ids may themselves contain slashes (nested refs).
            ("src_page_2/card/title", "dst_page_2/card/title"),
            // Bare component ids resolve by suffix at runtime — leave them.
            ("submit-button", "submit-button"),
            // Unknown heads are user data, not references.
            ("unknown_page/submit-button", "unknown_page/submit-button"),
            (
                "https://example.com/src_page/x",
                "https://example.com/src_page/x",
            ),
            ("src_page/", "src_page/"),
        ];

        for (input, expected) in cases {
            let mut default_value = serde_json::to_vec(&flow_like_types::json::json!(input))
                .expect("serialize pin default");
            rewrite_default_value_ids(&mut default_value, &maps);
            let decoded: flow_like_types::Value =
                serde_json::from_slice(&default_value).expect("decode pin default");
            assert_eq!(decoded, flow_like_types::json::json!(expected), "{input}");
        }
    }

    #[test]
    fn element_refs_nested_in_struct_pin_defaults_are_translated() {
        let mut maps = ForkIdMap::default();
        maps.pages
            .insert("src_page".to_string(), "dst_page".to_string());
        maps.nodes
            .insert("src_node".to_string(), "dst_node".to_string());

        let mut default_value = serde_json::to_vec(&flow_like_types::json::json!({
            "elementRef": "src_page/chart",
            "nodeId": "src_node",
            "label": "src_page is not a ref here",
        }))
        .expect("serialize pin default");
        rewrite_default_value_ids(&mut default_value, &maps);

        let decoded: flow_like_types::Value =
            serde_json::from_slice(&default_value).expect("decode pin default");
        assert_eq!(decoded["elementRef"], "dst_page/chart");
        assert_eq!(decoded["nodeId"], "dst_node");
        assert_eq!(decoded["label"], "src_page is not a ref here");
    }

    #[test]
    fn remapped_boards_keep_element_refs_pointing_at_the_forked_page() {
        let mut maps = ForkIdMap::default();
        maps.pages
            .insert("src_page".to_string(), "dst_page".to_string());

        let pin = proto::Pin {
            id: "src_pin".to_string(),
            name: "element_ref".to_string(),
            default_value: serde_json::to_vec(&flow_like_types::json::json!(
                "src_page/submit-button"
            ))
            .expect("serialize pin default"),
            ..Default::default()
        };
        let node = proto::Node {
            id: "src_node".to_string(),
            name: "a2ui_get_element".to_string(),
            pins: HashMap::from([(pin.id.clone(), pin)]),
            ..Default::default()
        };
        let board = proto::Board {
            id: "src_board".to_string(),
            page_ids: vec!["src_page".to_string()],
            nodes: HashMap::from([(node.id.clone(), node)]),
            ..Default::default()
        };

        let remapped = remap_board(board, &mut maps);

        assert_eq!(remapped.page_ids, vec!["dst_page".to_string()]);
        let pin = remapped
            .nodes
            .values()
            .next()
            .expect("node survives the remap")
            .pins
            .values()
            .next()
            .expect("pin survives the remap");
        let decoded: flow_like_types::Value =
            serde_json::from_slice(&pin.default_value).expect("decode pin default");
        assert_eq!(decoded, "dst_page/submit-button");
    }

    #[test]
    fn widget_selector_pin_defaults_follow_the_forked_widget_id() {
        let mut maps = ForkIdMap::default();
        maps.widgets
            .insert("src_widget".to_string(), "dst_widget".to_string());

        let cases = [
            // What the editor writes today: the bare project widget id.
            ("src_widget", "dst_widget"),
            // Package widgets are global — the package id must survive.
            (
                "pkg:com.example.sales/sales-chart",
                "pkg:com.example.sales/sales-chart",
            ),
            // Legacy boards stored the widget name; it is not a map key.
            ("Sales Chart", "Sales Chart"),
        ];

        for (input, expected) in cases {
            let mut default_value = serde_json::to_vec(&flow_like_types::json::json!(input))
                .expect("serialize pin default");
            rewrite_default_value_ids(&mut default_value, &maps);
            let decoded: flow_like_types::Value =
                serde_json::from_slice(&default_value).expect("decode pin default");
            assert_eq!(decoded, flow_like_types::json::json!(expected), "{input}");
        }
    }

    #[test]
    fn role_pin_defaults_translate_ids_but_not_role_names() {
        let mut maps = ForkIdMap::default();
        maps.roles
            .insert("src_role".to_string(), "dst_role".to_string());

        for (input, expected) in [("src_role", "dst_role"), ("Moderator", "Moderator")] {
            let mut default_value = serde_json::to_vec(&flow_like_types::json::json!(input))
                .expect("serialize pin default");
            rewrite_default_value_ids(&mut default_value, &maps);
            let decoded: flow_like_types::Value =
                serde_json::from_slice(&default_value).expect("decode pin default");
            assert_eq!(decoded, flow_like_types::json::json!(expected), "{input}");
        }
    }

    #[test]
    fn variable_ids_stay_out_of_the_pin_default_walk() {
        // remap_board preserves variable ids, so a var_ref default must keep
        // resolving against the unchanged board.variables keys.
        let mut maps = ForkIdMap::default();
        maps.variables
            .insert("src_var".to_string(), "dst_var".to_string());

        let mut default_value = serde_json::to_vec(&flow_like_types::json::json!("src_var"))
            .expect("serialize pin default");
        rewrite_default_value_ids(&mut default_value, &maps);
        let decoded: flow_like_types::Value =
            serde_json::from_slice(&default_value).expect("decode pin default");
        assert_eq!(decoded, "src_var");
    }

    #[test]
    fn uploaded_metadata_target_parses_supported_media_metadata_paths() {
        assert_eq!(
            UploadedMetadataTarget::from_relative_path("en.meta"),
            Some(UploadedMetadataTarget::App {
                lang: "en".to_string(),
            })
        );
        assert_eq!(
            UploadedMetadataTarget::from_relative_path("widgets/src_widget/de.meta"),
            Some(UploadedMetadataTarget::Widget {
                id: "src_widget".to_string(),
                lang: "de".to_string(),
            })
        );
        assert_eq!(
            UploadedMetadataTarget::from_relative_path("templates/src_template/fr.meta"),
            Some(UploadedMetadataTarget::Template {
                id: "src_template".to_string(),
                lang: "fr".to_string(),
            })
        );
    }

    #[test]
    fn relative_to_prefix_rejects_partial_prefix_matches() {
        assert_eq!(
            relative_to_prefix("media/apps/source/icon.webp", "media/apps/source"),
            Some("icon.webp".to_string())
        );
        assert_eq!(
            relative_to_prefix("media/apps/source-extra/icon.webp", "media/apps/source"),
            None
        );
    }

    #[test]
    fn join_relative_keeps_nested_paths_nested() {
        let prefix = Path::from("apps").child("dst").child("storage");

        assert_eq!(
            join_relative(&prefix, "db/tables.lance/data/chunk.lance").as_ref(),
            "apps/dst/storage/db/tables.lance/data/chunk.lance"
        );
        assert_eq!(
            join_relative(&prefix, "notes.txt").as_ref(),
            "apps/dst/storage/notes.txt"
        );
        assert_eq!(join_relative(&prefix, "").as_ref(), "apps/dst/storage");

        // The bug this guards against: `child` percent-encodes the
        // delimiter, flattening the whole relative path into one key.
        assert_ne!(
            prefix.child("db/tables.lance/data/chunk.lance").as_ref(),
            join_relative(&prefix, "db/tables.lance/data/chunk.lance").as_ref()
        );
    }

    #[tokio::test]
    async fn copy_object_prefix_mirrors_the_project_database_tree() {
        use flow_like_storage::object_store::{ObjectStore, memory::InMemory};

        // Same-deployment forks read and write through one store — mirror
        // that here so `copy_one` takes its native-copy path.
        let src: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
        let dst = src.clone();

        let sources = [
            "apps/src/storage/db/tables.lance/data/chunk.lance",
            "apps/src/storage/db/tables.lance/_versions/1.manifest",
            "apps/src/storage/db/tables.lance/_transactions/1-abc.txn",
            "apps/src/storage/db/tables.lance/_indices/idx/index.idx",
            "apps/src/storage/notes.txt",
        ];
        for path in sources {
            src.put(&Path::from(path), bytes::Bytes::from_static(b"x").into())
                .await
                .expect("seed source object");
        }

        let tally = copy_object_prefix(
            &src,
            &dst,
            &Path::from("apps/src/storage"),
            &Path::from("apps/dst/storage"),
            "app storage",
            None,
        )
        .await
        .expect("copy storage prefix");

        let mut copied: Vec<String> = futures::TryStreamExt::try_collect::<Vec<_>>(
            dst.list(Some(&Path::from("apps/dst/storage"))),
        )
        .await
        .expect("list destination")
        .into_iter()
        .map(|meta| meta.location.as_ref().to_string())
        .collect();
        copied.sort();

        assert_eq!(
            copied,
            vec![
                "apps/dst/storage/db/tables.lance/_indices/idx/index.idx".to_string(),
                "apps/dst/storage/db/tables.lance/_transactions/1-abc.txn".to_string(),
                "apps/dst/storage/db/tables.lance/_versions/1.manifest".to_string(),
                "apps/dst/storage/db/tables.lance/data/chunk.lance".to_string(),
                "apps/dst/storage/notes.txt".to_string(),
            ]
        );
        assert_eq!(tally.objects, 5);
        assert_eq!(tally.bytes, 5, "each seeded object is one byte");
    }

    /// The policy must never split a `{table}.lance/` directory: a table
    /// is only openable with `data/`, `_versions/`, `_transactions/` and
    /// `_indices/` all present. Schema-only drops whole user tables and
    /// keeps reserved artifact tables intact.
    #[tokio::test]
    async fn copy_object_prefix_honours_the_schema_only_skip_predicate() {
        use flow_like_storage::object_store::{ObjectStore, memory::InMemory};

        let src: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
        let dst = src.clone();

        let sources = [
            "apps/src/storage/db/tables.lance/data/chunk.lance",
            "apps/src/storage/db/tables.lance/_versions/1.manifest",
            "apps/src/storage/db/tables.lance/_transactions/1-abc.txn",
            "apps/src/storage/db/tables.lance/_indices/idx/index.idx",
            "apps/src/storage/db/__graph_overlays__.lance/data/chunk.lance",
            "apps/src/storage/db/__graph_overlays__.lance/_versions/1.manifest",
            "apps/src/storage/notes.txt",
            "apps/src/storage/node_scratch/out.json",
        ];
        for path in sources {
            src.put(&Path::from(path), bytes::Bytes::from_static(b"x").into())
                .await
                .expect("seed source object");
        }

        let policy = ForkPolicy {
            databases: ForkDatabaseMode::SchemaOnly,
            ..Default::default()
        };
        let skip = policy::storage_skip(&policy);
        copy_object_prefix(
            &src,
            &dst,
            &Path::from("apps/src/storage"),
            &Path::from("apps/dst/storage"),
            "app storage",
            skip.as_deref(),
        )
        .await
        .expect("copy storage prefix");

        let mut copied: Vec<String> = futures::TryStreamExt::try_collect::<Vec<_>>(
            dst.list(Some(&Path::from("apps/dst/storage"))),
        )
        .await
        .expect("list destination")
        .into_iter()
        .map(|meta| meta.location.as_ref().to_string())
        .collect();
        copied.sort();

        assert_eq!(
            copied,
            vec![
                "apps/dst/storage/db/__graph_overlays__.lance/_versions/1.manifest".to_string(),
                "apps/dst/storage/db/__graph_overlays__.lance/data/chunk.lance".to_string(),
                "apps/dst/storage/node_scratch/out.json".to_string(),
                "apps/dst/storage/notes.txt".to_string(),
            ],
            "user tables drop whole, reserved tables and flow scratch stay"
        );
    }

    #[tokio::test]
    async fn copy_object_prefix_skips_the_whole_database_when_excluded() {
        use flow_like_storage::object_store::{ObjectStore, memory::InMemory};

        let src: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
        let dst = src.clone();

        for path in [
            "apps/src/storage/db/tables.lance/data/chunk.lance",
            "apps/src/storage/db/__graph_overlays__.lance/data/chunk.lance",
            "apps/src/storage/notes.txt",
        ] {
            src.put(&Path::from(path), bytes::Bytes::from_static(b"x").into())
                .await
                .expect("seed source object");
        }

        let policy = ForkPolicy {
            databases: ForkDatabaseMode::None,
            ..Default::default()
        };
        let skip = policy::storage_skip(&policy);
        copy_object_prefix(
            &src,
            &dst,
            &Path::from("apps/src/storage"),
            &Path::from("apps/dst/storage"),
            "app storage",
            skip.as_deref(),
        )
        .await
        .expect("copy storage prefix");

        let copied: Vec<String> = futures::TryStreamExt::try_collect::<Vec<_>>(
            dst.list(Some(&Path::from("apps/dst/storage"))),
        )
        .await
        .expect("list destination")
        .into_iter()
        .map(|meta| meta.location.as_ref().to_string())
        .collect();

        assert_eq!(copied, vec!["apps/dst/storage/notes.txt".to_string()]);
    }

    #[tokio::test]
    async fn copy_metadata_with_translation_keeps_widget_subpaths_nested() {
        use flow_like_storage::object_store::{ObjectStore, memory::InMemory};

        let src: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
        let dst = src.clone();

        for path in [
            "apps/src/metadata/en.meta",
            "apps/src/metadata/widgets/src_widget/en.meta",
        ] {
            src.put(&Path::from(path), bytes::Bytes::from_static(b"x").into())
                .await
                .expect("seed source metadata");
        }

        let mut maps = ForkIdMap::default();
        maps.widgets
            .insert("src_widget".to_string(), "dst_widget".to_string());

        let shipped_widgets = HashSet::from(["src_widget".to_string()]);
        copy_metadata_with_translation(
            &src,
            &dst,
            &Path::from("apps/src/metadata"),
            &Path::from("apps/dst/metadata"),
            &maps,
            &shipped_widgets,
            &HashSet::new(),
        )
        .await
        .expect("copy metadata prefix");

        let mut copied: Vec<String> = futures::TryStreamExt::try_collect::<Vec<_>>(
            dst.list(Some(&Path::from("apps/dst/metadata"))),
        )
        .await
        .expect("list destination")
        .into_iter()
        .map(|meta| meta.location.as_ref().to_string())
        .collect();
        copied.sort();

        assert_eq!(
            copied,
            vec![
                "apps/dst/metadata/en.meta".to_string(),
                "apps/dst/metadata/widgets/dst_widget/en.meta".to_string(),
            ]
        );
    }

    /// Metadata for a category the policy excluded must be dropped, not
    /// translated — otherwise the fork carries rows describing widgets
    /// and templates that were never copied.
    #[tokio::test]
    async fn copy_metadata_with_translation_drops_metadata_for_unshipped_artifacts() {
        use flow_like_storage::object_store::{ObjectStore, memory::InMemory};

        let src: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
        let dst = src.clone();

        for path in [
            "apps/src/metadata/en.meta",
            "apps/src/metadata/widgets/src_widget/en.meta",
            "apps/src/metadata/templates/src_template/en.meta",
        ] {
            src.put(&Path::from(path), bytes::Bytes::from_static(b"x").into())
                .await
                .expect("seed source metadata");
        }

        let mut maps = ForkIdMap::default();
        maps.widgets
            .insert("src_widget".to_string(), "dst_widget".to_string());
        maps.templates
            .insert("src_template".to_string(), "dst_template".to_string());

        copy_metadata_with_translation(
            &src,
            &dst,
            &Path::from("apps/src/metadata"),
            &Path::from("apps/dst/metadata"),
            &maps,
            &HashSet::new(),
            &HashSet::new(),
        )
        .await
        .expect("copy metadata prefix");

        let copied: Vec<String> = futures::TryStreamExt::try_collect::<Vec<_>>(
            dst.list(Some(&Path::from("apps/dst/metadata"))),
        )
        .await
        .expect("list destination")
        .into_iter()
        .map(|meta| meta.location.as_ref().to_string())
        .collect();

        assert_eq!(
            copied,
            vec!["apps/dst/metadata/en.meta".to_string()],
            "app-level metadata always travels; excluded artifacts' metadata does not"
        );
    }

    #[test]
    fn selected_totals_drop_the_excluded_categories() {
        use crate::utils::fork::preview::{ForkCategorySize, ForkSizeBreakdown};

        let breakdown = ForkSizeBreakdown {
            always: ForkCategorySize {
                bytes: 10,
                objects: 1,
            },
            flows: ForkCategorySize {
                bytes: 20,
                objects: 2,
            },
            files: ForkCategorySize {
                bytes: 40,
                objects: 4,
            },
            databases: ForkCategorySize {
                bytes: 800,
                objects: 0,
            },
            widgets: ForkCategorySize {
                bytes: 5,
                objects: 1,
            },
            templates: ForkCategorySize {
                bytes: 5,
                objects: 1,
            },
        };

        assert_eq!(breakdown.total(), (880, 9));
        assert_eq!(breakdown.selected(&ForkPolicy::default()), (880, 9));

        // The product win: an app whose database blows the cap becomes
        // forkable once the owner excludes it.
        let no_db = ForkPolicy {
            databases: ForkDatabaseMode::None,
            ..Default::default()
        };
        assert_eq!(breakdown.selected(&no_db), (80, 9));

        // Schema-only ships no rows either, so it costs the same.
        let schema_only = ForkPolicy {
            databases: ForkDatabaseMode::SchemaOnly,
            ..Default::default()
        };
        assert_eq!(breakdown.selected(&schema_only), (80, 9));

        let minimal = ForkPolicy {
            flows: true,
            files: false,
            databases: ForkDatabaseMode::None,
            roles: false,
            widgets: false,
            templates: false,
        };
        assert_eq!(breakdown.selected(&minimal), (30, 3));
    }

    fn widget_fork_maps() -> ForkIdMap {
        let mut maps = ForkIdMap {
            source_app_id: "src_app".to_string(),
            app_id: "dst_app".to_string(),
            ..Default::default()
        };
        maps.nodes
            .insert("src_node".to_string(), "dst_node".to_string());
        maps.boards
            .insert("src_board".to_string(), "dst_board".to_string());
        maps.pages
            .insert("src_page".to_string(), "dst_page".to_string());
        maps.widgets
            .insert("src_widget".to_string(), "dst_widget".to_string());
        maps
    }

    fn json_bytes(value: flow_like_types::Value) -> Vec<u8> {
        flow_like_types::json::to_vec(&value).expect("serialize fixture")
    }

    fn decode_bytes(bytes: &[u8]) -> flow_like_types::Value {
        flow_like_types::json::from_slice(bytes).expect("decode fixture")
    }

    fn component_with_json(id: &str, value: flow_like_types::Value) -> proto::Component {
        proto::Component {
            id: id.to_string(),
            component_json: Some(json_bytes(value)),
            ..Default::default()
        }
    }

    fn literal(value: &str) -> proto::BoundValue {
        proto::BoundValue {
            value: Some(proto::bound_value::Value::LiteralString(value.to_string())),
            ..Default::default()
        }
    }

    fn literal_of(bound: &proto::BoundValue) -> &str {
        match bound.value.as_ref().expect("bound value present") {
            proto::bound_value::Value::LiteralString(value) => value.as_str(),
            other => panic!("expected a literal string, got {other:?}"),
        }
    }

    fn page_with_instances(instances: Vec<proto::WidgetInstance>) -> proto::Page {
        proto::Page {
            content: instances
                .into_iter()
                .map(|instance| proto::PageContent {
                    content_type: Some(proto::page_content::ContentType::Widget(instance)),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }
    }

    fn instance_at(page: &proto::Page, index: usize) -> &proto::WidgetInstance {
        match page.content[index].content_type.as_ref() {
            Some(proto::page_content::ContentType::Widget(instance)) => instance,
            other => panic!("expected a widget instance, got {other:?}"),
        }
    }

    #[test]
    fn widget_instance_references_follow_the_fork_maps() {
        let maps = widget_fork_maps();
        let mut binding = proto::ActionBinding {
            binding_type: Some(proto::action_binding::BindingType::WorkflowEventId(
                "src_node".to_string(),
            )),
            ..Default::default()
        };
        for (field, value) in [
            ("nodeId", "src_node"),
            ("boardId", "src_board"),
            ("appId", "src_app"),
            ("label", "src_node"),
        ] {
            binding
                .context_mapping
                .insert(field.to_string(), literal(value));
        }

        let mut instance = proto::WidgetInstance {
            widget_id: "src_widget".to_string(),
            instance_id: "instance".to_string(),
            widget_ref: Some(proto::WidgetRef {
                app_id: "src_app".to_string(),
                widget_id: "src_widget".to_string(),
                version: None,
            }),
            ..Default::default()
        };
        instance
            .action_bindings
            .insert("submit".to_string(), binding);
        // Keyed by prop id, so only whole-string matching can find these.
        instance.exposed_prop_values.insert(
            "target".to_string(),
            json_bytes(flow_like_types::json::json!({ "literalString": "src_page" })),
        );
        instance.customization_values.insert(
            "start".to_string(),
            json_bytes(flow_like_types::json::json!("src_node")),
        );
        instance.customization_values.insert(
            "title".to_string(),
            json_bytes(flow_like_types::json::json!("Ingest files")),
        );

        let mut page = page_with_instances(vec![instance]);
        let issues = remap_page(&mut page, "dst_page", &maps);
        assert!(issues.is_empty(), "{issues:?}");

        let instance = instance_at(&page, 0);
        let binding = &instance.action_bindings["submit"];
        assert_eq!(instance.widget_id, "dst_widget");
        assert_eq!(
            binding.binding_type,
            Some(proto::action_binding::BindingType::WorkflowEventId(
                "dst_node".to_string()
            ))
        );
        assert_eq!(literal_of(&binding.context_mapping["nodeId"]), "dst_node");
        assert_eq!(literal_of(&binding.context_mapping["boardId"]), "dst_board");
        assert_eq!(literal_of(&binding.context_mapping["appId"]), "dst_app");
        // A field the runtime does not treat as a reference keeps its value
        // even when that value happens to be a known id.
        assert_eq!(literal_of(&binding.context_mapping["label"]), "src_node");
        assert_eq!(
            decode_bytes(&instance.exposed_prop_values["target"])["literalString"],
            "dst_page"
        );
        assert_eq!(
            decode_bytes(&instance.customization_values["start"]),
            flow_like_types::Value::String("dst_node".to_string())
        );
        assert_eq!(
            decode_bytes(&instance.customization_values["title"]),
            flow_like_types::Value::String("Ingest files".to_string())
        );

        let widget_ref = instance.widget_ref.as_ref().expect("ref kept");
        assert_eq!(widget_ref.app_id, "dst_app");
        assert_eq!(widget_ref.widget_id, "dst_widget");
    }

    #[test]
    fn a_third_party_widget_ref_is_not_half_translated() {
        let maps = widget_fork_maps();
        let mut page = page_with_instances(vec![proto::WidgetInstance {
            widget_id: "src_widget".to_string(),
            instance_id: "b".to_string(),
            widget_ref: Some(proto::WidgetRef {
                app_id: "other_app".to_string(),
                widget_id: "src_widget".to_string(),
                version: None,
            }),
            ..Default::default()
        }]);

        remap_page(&mut page, "dst_page", &maps);

        // Translating half of the pair would address a widget id minted for
        // this fork inside an app that knows nothing about it.
        let widget_ref = instance_at(&page, 0).widget_ref.as_ref().expect("ref kept");
        assert_eq!(widget_ref.app_id, "other_app");
        assert_eq!(widget_ref.widget_id, "src_widget");
    }

    #[test]
    fn embedded_widget_definitions_are_remapped_whole() {
        let maps = widget_fork_maps();
        let mut page = proto::Page {
            board_id: Some("src_board".to_string()),
            ..Default::default()
        };
        let widget = proto::Widget {
            id: "src_widget".to_string(),
            components: vec![component_with_json(
                "button",
                flow_like_types::json::json!({
                    "actions": [{ "name": "workflow_event", "context": { "nodeId": "src_node" } }]
                }),
            )],
            exposed_props: vec![proto::ExposedProp {
                id: "target".to_string(),
                default_value: Some(json_bytes(
                    flow_like_types::json::json!({ "literalString": "src_page" }),
                )),
                ..Default::default()
            }],
            customization_options: vec![proto::CustomizationOption {
                id: "start".to_string(),
                default_value: Some(json_bytes(flow_like_types::json::json!("src_node"))),
                ..Default::default()
            }],
            data_model: vec![proto::DataEntry {
                key: "seed".to_string(),
                value: json_bytes(flow_like_types::json::json!({ "pageId": "src_page" })),
            }],
            ..Default::default()
        };
        // The map is keyed by widget *instance* id, not widget id.
        page.widget_refs.insert("slot-1".to_string(), widget);

        let issues = remap_page(&mut page, "dst_page", &maps);
        assert!(issues.is_empty(), "{issues:?}");

        assert!(page.widget_refs.contains_key("slot-1"));
        let widget = &page.widget_refs["slot-1"];
        assert_eq!(widget.id, "dst_widget");
        assert_eq!(
            decode_bytes(widget.components[0].component_json.as_ref().unwrap())["actions"][0]["context"]
                ["nodeId"],
            "dst_node"
        );
        assert_eq!(
            decode_bytes(widget.exposed_props[0].default_value.as_ref().unwrap())["literalString"],
            "dst_page"
        );
        assert_eq!(
            decode_bytes(
                widget.customization_options[0]
                    .default_value
                    .as_ref()
                    .unwrap()
            ),
            flow_like_types::Value::String("dst_node".to_string())
        );
        assert_eq!(
            decode_bytes(&widget.data_model[0].value)["pageId"],
            "dst_page"
        );
    }

    #[test]
    fn literal_json_action_targets_survive_the_fork() {
        let maps = widget_fork_maps();
        let mut page = proto::Page::default();
        page.components.push(component_with_json(
            "cta",
            flow_like_types::json::json!({
                "data": { "literalJson": "{\"pageId\":\"src_page\",\"label\":\"Open\"}" }
            }),
        ));

        let issues = remap_page(&mut page, "dst_page", &maps);
        assert!(issues.is_empty());

        let decoded = decode_bytes(page.components[0].component_json.as_ref().unwrap());
        let inner: flow_like_types::Value =
            flow_like_types::json::from_str(decoded["data"]["literalJson"].as_str().unwrap())
                .expect("literalJson stays parseable");
        assert_eq!(inner["pageId"], "dst_page");
        assert_eq!(inner["label"], "Open");
    }

    #[test]
    fn a_payload_the_fork_cannot_rewrite_is_reported_not_swallowed() {
        let maps = widget_fork_maps();
        let mut page = proto::Page::default();
        page.components.push(proto::Component {
            id: "broken".to_string(),
            component_json: Some(b"{not json".to_vec()),
            ..Default::default()
        });

        let issues = remap_page(&mut page, "dst_page", &maps);
        assert!(!issues.is_empty());
        assert!(
            issues.reason("page").contains("broken"),
            "the reason names the component: {}",
            issues.reason("page")
        );
    }
}
