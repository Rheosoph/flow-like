//! Duplicating an offline app on the device that holds it.
//!
//! The copy is a second, independent app: every board, node, pin, layer,
//! event, page, widget and template gets a fresh id through [`super::remap`] —
//! the same rewrite a fork uses — and every other file under the app (uploads,
//! the project database, media) is copied byte for byte. Nothing is stripped:
//! the copy stays on the same device with the same owner.
//!
//! Version history does not travel. Only the board versions an event is
//! pinned to are copied, because the event cannot run without them.

use super::{
    App,
    remap::{self, ForkIdMap},
};
use crate::{
    a2ui::widget::Page,
    state::FlowLikeState,
    utils::compression::{
        compress_to_file, compress_to_file_json, from_compressed, from_compressed_json,
    },
};
use flow_like_storage::{
    Path, join_object_path,
    object_store::{
        ObjectMeta, ObjectStore, ObjectStoreExt,
        buffered::{BufReader, BufWriter},
    },
};
use flow_like_types::{Timestamp, Value, anyhow, create_id, proto, tokio};
use futures::{StreamExt, TryStreamExt};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    sync::Arc,
    time::SystemTime,
};
use tokio::io::AsyncWriteExt;

const COPY_CONCURRENCY: usize = 16;

/// What [`App::duplicate_local`] produced.
#[derive(Clone, Debug, Serialize)]
pub struct DuplicateReport {
    pub app_id: String,
    pub source_app_id: String,
    /// Source board id → the copy's board id, for every board that was copied.
    /// Device-local state keyed by board (runtime variable values) follows
    /// the copy through it.
    pub board_ids: HashMap<String, String>,
    pub pages: usize,
    pub events: usize,
    pub widgets: usize,
    pub templates: usize,
    pub files_copied: u64,
    pub bytes_copied: u64,
    /// Artifacts that could not be copied or rewritten, one line each.
    pub warnings: Vec<String>,
}

impl App {
    /// Copy the offline app `source_app_id` into a new offline app on the same
    /// stores. `name` replaces the display name in every language. A failed
    /// duplicate removes whatever it had written; the source is never touched.
    pub async fn duplicate_local(
        app_state: Arc<FlowLikeState>,
        source_app_id: &str,
        name: Option<String>,
    ) -> flow_like_types::Result<DuplicateReport> {
        let meta = FlowLikeState::project_meta_store(&app_state)
            .await?
            .as_generic();
        let storage = FlowLikeState::project_storage_store(&app_state)
            .await?
            .as_generic();
        let mut duplication = Duplication::new(meta, storage, source_app_id, &create_id());
        let result = duplication.run(name).await;
        if result.is_err() {
            duplication.discard().await;
        }
        result
    }
}

/// One object under the source app. The desktop registers the same directory
/// as meta and storage store, so both listings name every file; each is copied
/// once, from the store its kind of data lives in.
struct SourceFile {
    in_storage: bool,
    meta: ObjectMeta,
}

struct CopiedBoard {
    source_id: String,
    source_page_ids: Vec<String>,
    board: proto::Board,
}

struct SourceTemplate {
    source_id: String,
    page_ids: Vec<String>,
    board: proto::Board,
}

/// Source ids of everything that made it into the copy.
#[derive(Default)]
struct Shipped {
    boards: HashSet<String>,
    pages: HashSet<String>,
    events: Vec<String>,
    widgets: HashSet<String>,
    templates: HashSet<String>,
}

struct Duplication {
    meta: Arc<dyn ObjectStore>,
    storage: Arc<dyn ObjectStore>,
    src: Path,
    dst: Path,
    maps: ForkIdMap,
    shipped: Shipped,
    warnings: Vec<String>,
}

impl Duplication {
    fn new(
        meta: Arc<dyn ObjectStore>,
        storage: Arc<dyn ObjectStore>,
        source_app_id: &str,
        app_id: &str,
    ) -> Self {
        Self {
            meta,
            storage,
            src: Path::from("apps").join(source_app_id),
            dst: Path::from("apps").join(app_id),
            maps: ForkIdMap {
                source_app_id: source_app_id.to_string(),
                app_id: app_id.to_string(),
                seed: app_id.to_string(),
                ..Default::default()
            },
            shipped: Shipped::default(),
            warnings: Vec::new(),
        }
    }

    async fn run(&mut self, name: Option<String>) -> flow_like_types::Result<DuplicateReport> {
        let manifest = self.read_manifest().await?;
        let files = self.list_sources().await?;
        let boards = self.load_boards(&manifest.boards).await;
        let templates = self.load_templates(&manifest.templates, &files).await;
        let event_ids = event_ids(&manifest, &files);
        self.allocate_ids(&manifest, &boards, &templates, &event_ids);

        let mut boards = self.remap_boards(boards)?;
        self.copy_board_pages(&mut boards).await?;
        for copied in &boards {
            self.write_proto(&format!("{}.board", copied.board.id), &copied.board)
                .await?;
            self.shipped.boards.insert(copied.source_id.clone());
        }

        let events = self.load_events(&event_ids).await;
        self.copy_pinned_versions(&events, &files).await?;
        self.write_events(events).await?;
        self.copy_widgets(&manifest.widget_ids).await?;
        self.copy_templates(templates).await?;
        let copied = self.copy_files(&files, name.as_deref()).await?;
        self.write_manifest(manifest).await?;
        Ok(self.report(copied))
    }

    async fn read_manifest(&self) -> flow_like_types::Result<proto::App> {
        let source_app_id = &self.maps.source_app_id;
        let manifest: proto::App =
            from_compressed(self.meta.clone(), self.src_path("manifest.app"))
                .await
                .map_err(|e| {
                    anyhow!(
                        "Cannot duplicate app {source_app_id}: its manifest could not be read: {e}"
                    )
                })?;
        if manifest.visibility != proto::AppVisibility::Offline as i32 {
            return Err(anyhow!(
                "Cannot duplicate app {source_app_id} on this device: only offline apps can be duplicated locally, fork online apps instead"
            ));
        }
        Ok(manifest)
    }

    fn report(&mut self, (files_copied, bytes_copied): (u64, u64)) -> DuplicateReport {
        DuplicateReport {
            app_id: self.maps.app_id.clone(),
            source_app_id: self.maps.source_app_id.clone(),
            board_ids: self
                .shipped
                .boards
                .iter()
                .map(|id| (id.clone(), self.maps.translate_board(id)))
                .collect(),
            pages: self.shipped.pages.len(),
            events: self.shipped.events.len(),
            widgets: self.shipped.widgets.len(),
            templates: self.shipped.templates.len(),
            files_copied,
            bytes_copied,
            warnings: std::mem::take(&mut self.warnings),
        }
    }

    async fn list_sources(&self) -> flow_like_types::Result<BTreeMap<String, SourceFile>> {
        let mut files = BTreeMap::new();
        for (store, in_storage) in [(&self.meta, false), (&self.storage, true)] {
            for (rel, meta) in list_prefix(store, &self.src).await? {
                let prefer = in_storage && is_content_path(&rel);
                if prefer || !files.contains_key(&rel) {
                    files.insert(rel, SourceFile { in_storage, meta });
                }
            }
        }
        Ok(files)
    }

    async fn load_boards(&mut self, ids: &[String]) -> Vec<(String, proto::Board)> {
        let mut boards = Vec::new();
        for id in unique(ids) {
            let rel = format!("{id}.board");
            if let Some(board) = self.read_or_warn(&rel, &format!("Board {id}")).await {
                boards.push((id.clone(), board));
            }
        }
        boards
    }

    async fn load_templates(
        &mut self,
        ids: &[String],
        files: &BTreeMap<String, SourceFile>,
    ) -> Vec<SourceTemplate> {
        let mut templates = Vec::new();
        for id in unique(ids) {
            let rel = format!("{id}.template");
            if let Some(board) = self.read_or_warn(&rel, &format!("Template {id}")).await {
                templates.push(SourceTemplate {
                    source_id: id.clone(),
                    page_ids: page_files(files, &format!("_template_{id}/")),
                    board,
                });
            }
        }
        templates
    }

    /// Every id is allocated before anything is rewritten, and every board's
    /// nodes are registered up front, so a reference into another board or a
    /// page resolves no matter which artifact is remapped first.
    fn allocate_ids(
        &mut self,
        manifest: &proto::App,
        boards: &[(String, proto::Board)],
        templates: &[SourceTemplate],
        event_ids: &[String],
    ) {
        let maps = &mut self.maps;
        allocate(
            &maps.seed,
            &mut maps.boards,
            boards.iter().map(|(id, _)| id),
        );
        allocate(&maps.seed, &mut maps.events, event_ids);
        allocate(
            &maps.seed,
            &mut maps.pages,
            manifest
                .page_ids
                .iter()
                .chain(boards.iter().flat_map(|(_, board)| &board.page_ids))
                .chain(templates.iter().flat_map(|t| &t.page_ids))
                .chain(templates.iter().flat_map(|t| &t.board.page_ids)),
        );
        allocate(&maps.seed, &mut maps.widgets, &manifest.widget_ids);
        allocate(
            &maps.seed,
            &mut maps.templates,
            templates.iter().map(|t| &t.source_id),
        );
        for (_, board) in boards {
            remap::register_board_ids(board, maps);
        }
        for template in templates {
            remap::register_board_ids(&template.board, maps);
        }
    }

    fn remap_boards(
        &mut self,
        boards: Vec<(String, proto::Board)>,
    ) -> flow_like_types::Result<Vec<CopiedBoard>> {
        boards
            .into_iter()
            .map(|(source_id, board)| {
                let source_page_ids = board.page_ids.clone();
                let board = remap::remap_board(board, &mut self.maps)
                    .map_err(|e| e.context(format!("Board {source_id} cannot be duplicated")))?;
                Ok(CopiedBoard {
                    source_id,
                    source_page_ids,
                    board,
                })
            })
            .collect()
    }

    /// Pages are written before their board so the board lists only the
    /// pages that actually made it into the copy.
    async fn copy_board_pages(
        &mut self,
        boards: &mut [CopiedBoard],
    ) -> flow_like_types::Result<()> {
        for copied in boards.iter_mut() {
            let mut written = HashSet::new();
            for source_page_id in &copied.source_page_ids {
                let Some(page) = self.read_page(&copied.source_id, source_page_id).await else {
                    self.warn(format!(
                        "Page {source_page_id} of board {} could not be read and was not copied",
                        copied.source_id
                    ));
                    continue;
                };
                let dir = format!("_{}", copied.board.id);
                written.insert(self.write_page(page, source_page_id, &dir).await?);
                self.shipped.pages.insert(source_page_id.clone());
            }
            copied.board.page_ids.retain(|id| written.contains(id));
        }
        Ok(())
    }

    /// Canonical board-scoped page first, then the legacy app-level JSON page
    /// that `Board::load_page` still falls back to.
    async fn read_page(&self, board_id: &str, page_id: &str) -> Option<proto::Page> {
        let canonical = self.src_path(&format!("_{board_id}/{page_id}.page"));
        if let Ok(page) = from_compressed::<proto::Page>(self.meta.clone(), canonical).await {
            return Some(page);
        }
        let legacy = self.src_path(&format!("{page_id}.page"));
        from_compressed_json::<Page>(self.meta.clone(), legacy)
            .await
            .ok()
            .map(Into::into)
    }

    /// Moves `page` into the copy's id space and writes it into `dir`.
    /// Returns the page's new id.
    async fn write_page(
        &mut self,
        mut page: proto::Page,
        source_page_id: &str,
        dir: &str,
    ) -> flow_like_types::Result<String> {
        let page_id = self.maps.translate_page(source_page_id);
        for issue in remap::remap_page(&mut page, &page_id, &self.maps) {
            self.warn(format!(
                "Page {source_page_id} kept a reference to the original app: {issue}"
            ));
        }
        self.write_proto(&format!("{dir}/{page_id}.page"), &page)
            .await?;
        Ok(page_id)
    }

    /// Reads a compressed proto from the source app, or records that `what`
    /// was not copied.
    async fn read_or_warn<M: flow_like_types::Message + Default>(
        &mut self,
        rel: &str,
        what: &str,
    ) -> Option<M> {
        match from_compressed::<M>(self.meta.clone(), self.src_path(rel)).await {
            Ok(message) => Some(message),
            Err(e) => {
                self.warn(format!("{what} could not be read and was not copied: {e}"));
                None
            }
        }
    }

    async fn load_events(&mut self, ids: &[String]) -> Vec<(String, proto::Event)> {
        let mut events = Vec::new();
        for id in ids {
            let rel = format!("events/{id}.event");
            if let Some(event) = self.read_or_warn(&rel, &format!("Event {id}")).await {
                events.push((id.clone(), event));
            }
        }
        events
    }

    /// Runs before the events are written: a pinned version can hold nodes
    /// the live board no longer has, and an event targeting one of them is
    /// only copyable once that version registered its nodes.
    async fn copy_pinned_versions(
        &mut self,
        events: &[(String, proto::Event)],
        files: &BTreeMap<String, SourceFile>,
    ) -> flow_like_types::Result<()> {
        for (source_board_id, version) in pinned_versions(events) {
            if self.shipped.boards.contains(&source_board_id) {
                self.copy_pinned_version(&source_board_id, &version, files)
                    .await?;
            }
        }
        Ok(())
    }

    async fn copy_pinned_version(
        &mut self,
        source_board_id: &str,
        version: &str,
        files: &BTreeMap<String, SourceFile>,
    ) -> flow_like_types::Result<()> {
        let version_dir = format!("versions/{source_board_id}/{version}");
        let label = format!("Version {version} of board {source_board_id}");
        let Some(board) = self
            .read_or_warn::<proto::Board>(&format!("{version_dir}.board"), &label)
            .await
        else {
            return Ok(());
        };

        let page_ids = page_files(files, &format!("{version_dir}/"));
        let maps = &mut self.maps;
        allocate(
            &maps.seed,
            &mut maps.pages,
            board.page_ids.iter().chain(&page_ids),
        );
        let mut board = remap::remap_board(board, &mut self.maps)
            .map_err(|e| e.context(format!("{label} cannot be duplicated")))?;
        board.id = self.maps.translate_board(source_board_id);
        let dst_version_dir = format!("versions/{}/{version}", board.id);

        for source_page_id in &page_ids {
            let rel = format!("{version_dir}/{source_page_id}.page");
            let what = format!("Page {source_page_id} of {label}");
            if let Some(page) = self.read_or_warn(&rel, &what).await {
                self.write_page(page, source_page_id, &dst_version_dir)
                    .await?;
            }
        }
        self.write_proto(&format!("{dst_version_dir}.board"), &board)
            .await
    }

    async fn write_events(
        &mut self,
        events: Vec<(String, proto::Event)>,
    ) -> flow_like_types::Result<()> {
        for (source_id, mut event) in events {
            if !self.target_copied(&event.board_id, &event.node_id) {
                self.warn(format!(
                    "Event {source_id} targets a board or node that was not copied, so it was left out"
                ));
                continue;
            }
            self.prune_uncopied_targets(&source_id, &mut event);
            remap::remap_event(&mut event, &self.maps);
            self.write_proto(&format!("events/{}.event", event.id), &event)
                .await?;
            self.shipped.events.push(source_id);
        }
        Ok(())
    }

    /// A canary, variant or default page pointing outside the copy would
    /// keep the source's id, so it is dropped instead.
    fn prune_uncopied_targets(&mut self, source_id: &str, event: &mut proto::Event) {
        if event
            .canary
            .as_ref()
            .is_some_and(|canary| !self.target_copied(&canary.board_id, &canary.node_id))
        {
            self.warn(format!(
                "The canary of event {source_id} targets a board or node that was not copied and was removed"
            ));
            event.canary = None;
        }
        let dropped: Vec<String> = event
            .variants
            .iter()
            .filter(|variant| !self.target_copied(&variant.board_id, &variant.node_id))
            .map(|variant| variant.name.clone())
            .collect();
        for variant in &dropped {
            self.warn(format!(
                "Variant '{variant}' of event {source_id} targets a board or node that was not copied and was removed"
            ));
        }
        event
            .variants
            .retain(|variant| !dropped.contains(&variant.name));

        let pages = &self.shipped.pages;
        let uncopied = |page: &Option<String>| page.as_ref().is_some_and(|p| !pages.contains(p));
        if uncopied(&event.default_page_id) {
            event.default_page_id = None;
        }
        for variant in event.variants.iter_mut() {
            if uncopied(&variant.default_page_id) {
                variant.default_page_id = None;
            }
        }
    }

    fn target_copied(&self, board_id: &str, node_id: &str) -> bool {
        (board_id.is_empty() || self.shipped.boards.contains(board_id))
            && (node_id.is_empty() || self.maps.nodes.contains_key(node_id))
    }

    async fn copy_widgets(&mut self, ids: &[String]) -> flow_like_types::Result<()> {
        for source_id in unique(ids) {
            let path = self.src_path(&format!("{source_id}.widget"));
            let mut widget: Value = match from_compressed_json(self.meta.clone(), path).await {
                Ok(widget) => widget,
                Err(e) => {
                    self.warn(format!(
                        "Widget {source_id} could not be read and was not copied: {e}"
                    ));
                    continue;
                }
            };
            let widget_id = self.maps.widgets[source_id].clone();
            if let Some(object) = widget.as_object_mut() {
                object.insert("id".to_string(), Value::String(widget_id.clone()));
            }
            for issue in remap::remap_widget_json(&mut widget, &self.maps) {
                self.warn(format!(
                    "Widget {source_id} kept a reference to the original app: {issue}"
                ));
            }
            compress_to_file_json(
                self.meta.clone(),
                self.dst_path(&format!("{widget_id}.widget")),
                &widget,
            )
            .await?;
            self.shipped.widgets.insert(source_id.clone());
        }
        Ok(())
    }

    async fn copy_templates(
        &mut self,
        templates: Vec<SourceTemplate>,
    ) -> flow_like_types::Result<()> {
        for template in templates {
            let source_id = template.source_id;
            let template_id = self.maps.templates[&source_id].clone();
            let mut board = remap::remap_board(template.board, &mut self.maps)
                .map_err(|e| e.context(format!("Template {source_id} cannot be duplicated")))?;
            board.id = template_id.clone();

            let mut written = HashSet::new();
            let dir = format!("_template_{template_id}");
            for source_page_id in &template.page_ids {
                let rel = format!("_template_{source_id}/{source_page_id}.page");
                let what = format!("Page {source_page_id} of template {source_id}");
                if let Some(page) = self.read_or_warn(&rel, &what).await {
                    written.insert(self.write_page(page, source_page_id, &dir).await?);
                    self.shipped.pages.insert(source_page_id.clone());
                }
            }
            board.page_ids.retain(|id| written.contains(id));
            self.write_proto(&format!("{template_id}.template"), &board)
                .await?;
            self.shipped.templates.insert(source_id);
        }
        Ok(())
    }

    /// Everything that is not a rewritten artifact or version history: app
    /// metadata (renamed), per-widget/template/page metadata (re-keyed), and
    /// the rest byte for byte. Copies are real copies, never hard links — the
    /// app may hold files something writes in place, such as a SQLite file.
    async fn copy_files(
        &mut self,
        files: &BTreeMap<String, SourceFile>,
        name: Option<&str>,
    ) -> flow_like_types::Result<(u64, u64)> {
        let mut copies = Vec::new();
        for (rel, file) in files {
            if is_app_metadata(rel) && self.write_app_metadata(rel, file, name).await {
                continue;
            }
            let Some(target) = self.target_path(rel) else {
                continue;
            };
            copies.push(copy_object(
                self.store(file.in_storage),
                file.meta.clone(),
                self.dst_path(&target),
            ));
        }

        // The futures are built up front: a mapping closure over
        // `Arc<dyn ObjectStore>` makes this future fail the `Send` proof
        // Tauri commands need.
        futures::stream::iter(copies)
            .buffer_unordered(COPY_CONCURRENCY)
            .try_fold((0u64, 0u64), |(files, bytes), copied| async move {
                Ok(match copied {
                    Some(size) => (files + 1, bytes + size),
                    None => (files, bytes),
                })
            })
            .await
    }

    /// Returns whether the metadata was written; an undecodable file falls
    /// back to a byte copy.
    async fn write_app_metadata(
        &mut self,
        rel: &str,
        file: &SourceFile,
        name: Option<&str>,
    ) -> bool {
        let store = self.store(file.in_storage);
        let mut metadata: proto::Metadata =
            match from_compressed(store.clone(), file.meta.location.clone()).await {
                Ok(metadata) => metadata,
                Err(e) => {
                    self.warn(format!(
                        "App metadata {rel} could not be read and was copied as is: {e}"
                    ));
                    return false;
                }
            };
        if let Some(name) = name {
            metadata.name = name.to_string();
        }
        let now = Timestamp::from(SystemTime::now());
        metadata.created_at = Some(now);
        metadata.updated_at = Some(now);
        match compress_to_file(store, self.dst_path(rel), &metadata).await {
            Ok(_) => true,
            Err(e) => {
                self.warn(format!(
                    "App metadata {rel} could not be written and was copied as is: {e}"
                ));
                false
            }
        }
    }

    /// Where a copied file lands, or `None` when it is not copied at all.
    fn target_path(&self, rel: &str) -> Option<String> {
        if let Some(rest) = rel.strip_prefix("metadata/") {
            return self
                .translate_metadata_path(rest)
                .map(|translated| format!("metadata/{translated}"));
        }
        (!is_rewritten_or_history(rel)).then(|| rel.to_string())
    }

    /// `metadata/{widgets|templates|pages}/{id}/…` follows its owner into the
    /// new id space; metadata of an owner that was not copied stays behind.
    fn translate_metadata_path(&self, rest: &str) -> Option<String> {
        let mut segments = rest.splitn(3, '/');
        let kind = segments.next()?;
        let (Some(id), Some(tail)) = (segments.next(), segments.next()) else {
            return Some(rest.to_string());
        };
        let (map, shipped) = match kind {
            "widgets" => (&self.maps.widgets, &self.shipped.widgets),
            "templates" => (&self.maps.templates, &self.shipped.templates),
            "pages" => (&self.maps.pages, &self.shipped.pages),
            _ => return Some(rest.to_string()),
        };
        if !shipped.contains(id) {
            return None;
        }
        Some(format!("{kind}/{}/{tail}", map.get(id)?))
    }

    /// Written last: without a manifest the copy is not an app, so an
    /// interrupted duplicate never shows up half-done.
    async fn write_manifest(&mut self, mut manifest: proto::App) -> flow_like_types::Result<()> {
        let maps = &self.maps;
        let shipped = &self.shipped;

        manifest.id = maps.app_id.clone();
        manifest.boards = translate_copied(&manifest.boards, &shipped.boards, &maps.boards);
        manifest.page_ids = translate_copied(&manifest.page_ids, &shipped.pages, &maps.pages);
        manifest.widget_ids =
            translate_copied(&manifest.widget_ids, &shipped.widgets, &maps.widgets);
        manifest.templates =
            translate_copied(&manifest.templates, &shipped.templates, &maps.templates);
        manifest.events = shipped
            .events
            .iter()
            .map(|id| maps.translate_event(id))
            .collect();
        let copied_events: HashSet<&String> = shipped.events.iter().collect();
        manifest.route_mappings = manifest
            .route_mappings
            .iter()
            .filter(|(_, event_id)| copied_events.contains(event_id))
            .map(|(path, event_id)| (path.clone(), maps.translate_event(event_id)))
            .collect();

        let now = Timestamp::from(SystemTime::now());
        manifest.visibility = proto::AppVisibility::Offline as i32;
        manifest.status = proto::AppStatus::Active as i32;
        manifest.created_at = Some(now);
        manifest.updated_at = Some(now);
        manifest.allow_forking = Some(false);
        manifest.rating_sum = 0;
        manifest.rating_count = 0;
        manifest.download_count = 0;
        manifest.interaction_count = 0;
        manifest.avg_rating = None;
        manifest.relevance_score = None;

        self.write_proto("manifest.app", &manifest).await
    }

    async fn discard(&self) {
        for store in [&self.meta, &self.storage] {
            let Ok(entries) = list_prefix(store, &self.dst).await else {
                continue;
            };
            for (_, meta) in entries {
                match store.delete(&meta.location).await {
                    Ok(()) | Err(flow_like_storage::object_store::Error::NotFound { .. }) => {}
                    Err(e) => tracing::warn!(
                        "Failed to remove {} of a failed duplicate: {e}",
                        meta.location
                    ),
                }
            }
        }
    }

    async fn write_proto<M: flow_like_types::Message>(
        &self,
        rel: &str,
        message: &M,
    ) -> flow_like_types::Result<()> {
        compress_to_file(self.meta.clone(), self.dst_path(rel), message).await?;
        Ok(())
    }

    fn store(&self, in_storage: bool) -> Arc<dyn ObjectStore> {
        if in_storage {
            self.storage.clone()
        } else {
            self.meta.clone()
        }
    }

    fn src_path(&self, rel: &str) -> Path {
        join_object_path(&self.src, rel)
    }

    fn dst_path(&self, rel: &str) -> Path {
        join_object_path(&self.dst, rel)
    }

    fn warn(&mut self, message: String) {
        tracing::warn!("duplicate {}: {message}", self.maps.source_app_id);
        self.warnings.push(message);
    }
}

async fn list_prefix(
    store: &Arc<dyn ObjectStore>,
    prefix: &Path,
) -> flow_like_types::Result<Vec<(String, ObjectMeta)>> {
    let prefix_str = format!("{}/", prefix.as_ref());
    let mut entries = Vec::new();
    let mut listing = store.list(Some(prefix));
    loop {
        match listing.try_next().await {
            Ok(Some(meta)) => {
                if let Some(rel) = meta.location.as_ref().strip_prefix(&prefix_str)
                    && !rel.is_empty()
                {
                    entries.push((rel.to_string(), meta));
                }
            }
            Ok(None) => break,
            Err(flow_like_storage::object_store::Error::NotFound { .. }) => break,
            Err(e) => return Err(anyhow!("Failed to list {prefix}: {e}")),
        }
    }
    Ok(entries)
}

/// Streams the object so a multi-gigabyte database file never sits in
/// memory. `None` when the source vanished between listing and copying.
async fn copy_object(
    store: Arc<dyn ObjectStore>,
    meta: ObjectMeta,
    to: Path,
) -> flow_like_types::Result<Option<u64>> {
    let mut reader = BufReader::new(store.clone(), &meta);
    let mut writer = BufWriter::new(store, to.clone());
    match tokio::io::copy(&mut reader, &mut writer).await {
        Ok(bytes) => {
            writer
                .shutdown()
                .await
                .map_err(|e| anyhow!("Failed to write {to}: {e}"))?;
            Ok(Some(bytes))
        }
        Err(error) => {
            let _ = writer.abort().await;
            if error.kind() == std::io::ErrorKind::NotFound {
                return Ok(None);
            }
            Err(anyhow!("Failed to copy {} to {to}: {error}", meta.location))
        }
    }
}

fn allocate<'a>(
    seed: &str,
    map: &mut HashMap<String, String>,
    ids: impl IntoIterator<Item = &'a String>,
) {
    for id in ids {
        map.entry(id.clone())
            .or_insert_with(|| remap::derive_id(seed, id));
    }
}

/// The copy's ids for the entries of `ids` that were copied, in source order.
fn translate_copied(
    ids: &[String],
    copied: &HashSet<String>,
    map: &HashMap<String, String>,
) -> Vec<String> {
    unique(ids)
        .into_iter()
        .filter(|id| copied.contains(*id))
        .filter_map(|id| map.get(id).cloned())
        .collect()
}

fn unique(ids: &[String]) -> Vec<&String> {
    let mut seen = HashSet::new();
    ids.iter().filter(|id| seen.insert(*id)).collect()
}

/// The manifest's events plus any event file it does not list — the events
/// directory is what the desktop actually runs from.
fn event_ids(manifest: &proto::App, files: &BTreeMap<String, SourceFile>) -> Vec<String> {
    let on_disk = files.keys().filter_map(|rel| {
        rel.strip_prefix("events/")?
            .strip_suffix(".event")
            .filter(|id| !id.contains('/'))
            .map(str::to_string)
    });
    let ids: Vec<String> = manifest.events.iter().cloned().chain(on_disk).collect();
    unique(&ids).into_iter().cloned().collect()
}

/// Page ids of the `{dir}{page_id}.page` files directly inside `dir`.
fn page_files(files: &BTreeMap<String, SourceFile>, dir: &str) -> Vec<String> {
    files
        .keys()
        .filter_map(|rel| {
            rel.strip_prefix(dir)?
                .strip_suffix(".page")
                .filter(|id| !id.contains('/'))
                .map(str::to_string)
        })
        .collect()
}

/// `(board_id, "major_minor_patch")` for every board version an event, its
/// canary or one of its variants is pinned to.
fn pinned_versions(events: &[(String, proto::Event)]) -> BTreeSet<(String, String)> {
    let label = |v: &proto::Version| format!("{}_{}_{}", v.major, v.minor, v.patch);
    let mut pinned = BTreeSet::new();
    for (_, event) in events {
        if let Some(version) = event.board_version.as_ref() {
            pinned.insert((event.board_id.clone(), label(version)));
        }
        if let Some(canary) = event.canary.as_ref()
            && let Some(version) = canary.board_version.as_ref()
        {
            pinned.insert((canary.board_id.clone(), label(version)));
        }
        for variant in &event.variants {
            if let Some(version) = variant.board_version.as_ref() {
                pinned.insert((variant.board_id.clone(), label(version)));
            }
        }
    }
    pinned
}

fn is_content_path(rel: &str) -> bool {
    let head = rel.split('/').next().unwrap_or(rel);
    matches!(head, "metadata" | "media" | "upload" | "storage")
}

fn is_app_metadata(rel: &str) -> bool {
    rel.strip_prefix("metadata/")
        .is_some_and(|name| !name.contains('/') && name.ends_with(".meta"))
}

/// Artifacts the duplicate rewrites itself (or deliberately leaves behind as
/// version history) and must therefore never byte-copy: a copied `.board`
/// would resurrect the source's ids inside the new app.
fn is_rewritten_or_history(rel: &str) -> bool {
    const ARTIFACTS: [&str; 5] = [".board", ".page", ".event", ".widget", ".template"];
    let Some((head, _)) = rel.split_once('/') else {
        return rel.starts_with("manifest.app") || ARTIFACTS.iter().any(|ext| rel.ends_with(ext));
    };
    head.starts_with('_')
        || matches!(head, "events" | "versions")
        || rel.starts_with("templates/versions/")
        || rel.starts_with("widgets/versions/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        state::{FlowLikeConfig, FlowLikeState},
        utils::http::HTTPClient,
    };
    use flow_like_storage::{
        files::store::FlowLikeStore,
        object_store::{PutPayload, memory::InMemory},
    };

    const SRC: &str = "src_app";

    fn setup_state() -> (Arc<FlowLikeState>, Arc<InMemory>, Arc<InMemory>) {
        let mut config = FlowLikeConfig::new();
        let meta = Arc::new(InMemory::new());
        let storage = Arc::new(InMemory::new());
        config.register_app_meta_store(FlowLikeStore::Memory(meta.clone()));
        config.register_app_storage_store(FlowLikeStore::Memory(storage.clone()));
        let state = FlowLikeState::new(config, HTTPClient::new_without_refetch());
        (Arc::new(state), meta, storage)
    }

    fn path(app_id: &str, rel: &str) -> Path {
        join_object_path(&Path::from("apps").join(app_id), rel)
    }

    async fn put_proto<M: flow_like_types::Message>(store: &Arc<InMemory>, rel: &str, message: &M) {
        compress_to_file(store.clone(), path(SRC, rel), message)
            .await
            .unwrap();
    }

    async fn read_proto<M: flow_like_types::Message + Default>(
        store: &Arc<InMemory>,
        app_id: &str,
        rel: &str,
    ) -> M {
        from_compressed(store.clone(), path(app_id, rel))
            .await
            .unwrap()
    }

    fn pin(id: &str, connected_to: &[&str], default: Option<Value>) -> proto::Pin {
        proto::Pin {
            id: id.into(),
            name: id.into(),
            connected_to: connected_to.iter().map(|p| p.to_string()).collect(),
            default_value: default
                .map(|value| serde_json::to_vec(&value).unwrap())
                .unwrap_or_default(),
            ..Default::default()
        }
    }

    fn node(id: &str, pins: Vec<proto::Pin>) -> proto::Node {
        proto::Node {
            id: id.into(),
            name: id.into(),
            pins: pins.into_iter().map(|p| (p.id.clone(), p)).collect(),
            ..Default::default()
        }
    }

    fn secret(value: &[u8]) -> proto::Variable {
        proto::Variable {
            id: "api_key".into(),
            name: "api_key".into(),
            secret: true,
            default_value: value.to_vec(),
            ..Default::default()
        }
    }

    /// Two boards that reference each other, a page, an event pinned to a
    /// version, a widget, a template, uploads and a project database file.
    async fn seed(meta: &Arc<InMemory>, storage: &Arc<InMemory>) {
        let main = proto::Board {
            id: "board_main".into(),
            page_ids: vec!["page_home".into(), "page_missing".into()],
            nodes: HashMap::from([
                (
                    "node_event".into(),
                    node("node_event", vec![pin("pin_out", &["pin_in"], None)]),
                ),
                (
                    "node_call".into(),
                    node(
                        "node_call",
                        vec![
                            pin("pin_in", &["pin_out"], None),
                            pin("pin_target", &[], Some(Value::String("node_helper".into()))),
                        ],
                    ),
                ),
            ]),
            variables: HashMap::from([("api_key".into(), secret(b"shh"))]),
            ..Default::default()
        };
        let helper = proto::Board {
            id: "board_helper".into(),
            nodes: HashMap::from([("node_helper".into(), node("node_helper", vec![]))]),
            ..Default::default()
        };
        put_proto(meta, "board_main.board", &main).await;
        put_proto(meta, "board_helper.board", &helper).await;
        put_proto(meta, "versions/board_main/1_0_0.board", &main).await;
        put_proto(meta, "versions/board_main/0_9_0.board", &main).await;

        let page = proto::Page {
            id: "page_home".into(),
            board_id: Some("board_main".into()),
            on_load_event_id: Some("node_event".into()),
            ..Default::default()
        };
        put_proto(meta, "_board_main/page_home.page", &page).await;

        let event = proto::Event {
            id: "event_run".into(),
            board_id: "board_main".into(),
            node_id: "node_event".into(),
            board_version: Some(proto::Version {
                major: 1,
                minor: 0,
                patch: 0,
            }),
            default_page_id: Some("page_home".into()),
            ..Default::default()
        };
        put_proto(meta, "events/event_run.event", &event).await;
        let orphan = proto::Event {
            id: "event_orphan".into(),
            board_id: "board_gone".into(),
            ..Default::default()
        };
        put_proto(meta, "events/event_orphan.event", &orphan).await;
        put_proto(meta, "events/versions/event_run/0.0.0", &event).await;

        compress_to_file_json(
            meta.clone(),
            path(SRC, "widget_card.widget"),
            &flow_like_types::json::json!({ "id": "widget_card", "name": "Card" }),
        )
        .await
        .unwrap();

        let template = proto::Board {
            id: "template_a".into(),
            page_ids: vec!["template_page".into()],
            ..Default::default()
        };
        put_proto(meta, "template_a.template", &template).await;
        let template_page = proto::Page {
            id: "template_page".into(),
            board_id: Some("template_a".into()),
            ..Default::default()
        };
        put_proto(
            meta,
            "_template_template_a/template_page.page",
            &template_page,
        )
        .await;

        let manifest = proto::App {
            id: SRC.into(),
            visibility: proto::AppVisibility::Offline as i32,
            boards: vec![
                "board_main".into(),
                "board_helper".into(),
                "board_gone".into(),
            ],
            events: vec!["event_run".into()],
            page_ids: vec!["page_home".into()],
            widget_ids: vec!["widget_card".into()],
            templates: vec!["template_a".into()],
            route_mappings: HashMap::from([("/".into(), "event_run".into())]),
            ..Default::default()
        };
        put_proto(meta, "manifest.app", &manifest).await;

        let metadata = proto::Metadata {
            name: "Original".into(),
            icon: Some("icon_id".into()),
            ..Default::default()
        };
        compress_to_file(storage.clone(), path(SRC, "metadata/en.meta"), &metadata)
            .await
            .unwrap();
        compress_to_file(
            storage.clone(),
            path(SRC, "metadata/widgets/widget_card/en.meta"),
            &metadata,
        )
        .await
        .unwrap();
        for (rel, bytes) in [
            ("media/icon_id.webp", b"webp".to_vec()),
            ("upload/rpa/board_main/screenshots/a b.png", b"png".to_vec()),
            ("storage/db/items.lance/data/0.lance", vec![7u8; 64 * 1024]),
            ("storage/feedback.db", b"sqlite".to_vec()),
        ] {
            storage
                .put(&path(SRC, rel), PutPayload::from(bytes))
                .await
                .unwrap();
        }
    }

    async fn keys(store: &Arc<InMemory>, app_id: &str) -> BTreeSet<String> {
        let store: Arc<dyn ObjectStore> = store.clone();
        list_prefix(&store, &Path::from("apps").join(app_id))
            .await
            .unwrap()
            .into_iter()
            .map(|(rel, _)| rel)
            .collect()
    }

    #[tokio::test]
    async fn duplicate_rekeys_every_artifact_and_copies_all_content() {
        let (state, meta, storage) = setup_state();
        seed(&meta, &storage).await;
        let source_meta = keys(&meta, SRC).await;
        let source_storage = keys(&storage, SRC).await;

        let report = App::duplicate_local(state, SRC, Some("Copy".into()))
            .await
            .unwrap();
        let dst = report.app_id.as_str();
        assert_ne!(dst, SRC);
        assert_eq!(keys(&meta, SRC).await, source_meta);
        assert_eq!(keys(&storage, SRC).await, source_storage);
        assert_eq!(
            (
                report.board_ids.len(),
                report.pages,
                report.events,
                report.widgets,
                report.templates
            ),
            (2, 2, 1, 1, 1)
        );

        let manifest: proto::App = read_proto(&meta, dst, "manifest.app").await;
        assert_eq!(manifest.id, dst);
        assert_eq!(manifest.visibility, proto::AppVisibility::Offline as i32);
        assert_eq!(manifest.boards.len(), 2);
        let main_id = manifest.boards[0].clone();
        let helper_id = manifest.boards[1].clone();
        assert!(!manifest.boards.iter().any(|id| id.starts_with("board_")));

        let main: proto::Board = read_proto(&meta, dst, &format!("{main_id}.board")).await;
        assert_eq!(main.id, main_id);
        assert_eq!(main.page_ids.len(), 1, "unreadable page is dropped");
        assert_eq!(main.variables["api_key"].default_value, b"shh".to_vec());
        let helper: proto::Board = read_proto(&meta, dst, &format!("{helper_id}.board")).await;
        let helper_node = helper.nodes.keys().next().unwrap().clone();
        let call = main
            .nodes
            .values()
            .find(|node| node.pins.len() == 2)
            .unwrap();
        let target = call
            .pins
            .values()
            .find(|pin| pin.name == "pin_target")
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&target.default_value).unwrap(),
            Value::String(helper_node),
            "cross-board node references follow the copy"
        );
        let wired = call.pins.values().find(|pin| pin.name == "pin_in").unwrap();
        assert!(
            main.nodes
                .values()
                .any(|node| node.pins.contains_key(&wired.connected_to[0]))
        );

        let page_id = main.page_ids[0].clone();
        let page: proto::Page = read_proto(&meta, dst, &format!("_{main_id}/{page_id}.page")).await;
        assert_eq!(page.id, page_id);
        assert_eq!(page.board_id.as_deref(), Some(main_id.as_str()));
        assert!(
            main.nodes
                .contains_key(page.on_load_event_id.as_deref().unwrap())
        );

        let event_id = manifest.events[0].clone();
        let event: proto::Event = read_proto(&meta, dst, &format!("events/{event_id}.event")).await;
        assert_eq!(event.board_id, main_id);
        assert!(main.nodes.contains_key(&event.node_id));
        assert_eq!(event.default_page_id.as_deref(), Some(page_id.as_str()));
        assert_eq!(manifest.route_mappings["/"], event_id);

        let copied_meta = keys(&meta, dst).await;
        assert!(copied_meta.contains(&format!("versions/{main_id}/1_0_0.board")));
        assert!(
            !copied_meta.iter().any(|rel| rel.contains("0_9_0")),
            "unpinned history stays behind"
        );
        assert!(
            !copied_meta
                .iter()
                .any(|rel| rel.starts_with("events/versions/"))
        );
        assert!(
            !copied_meta.iter().any(|rel| rel.contains("src_")
                || rel.contains("board_")
                || rel.contains("widget_card")
                || rel.contains("template_a")),
            "no source id survives in a path: {copied_meta:?}"
        );

        let widget_id = manifest.widget_ids[0].clone();
        let widget: Value =
            from_compressed_json(meta.clone(), path(dst, &format!("{widget_id}.widget")))
                .await
                .unwrap();
        assert_eq!(widget["id"], widget_id.as_str());
        let template_id = manifest.templates[0].clone();
        let template: proto::Board =
            read_proto(&meta, dst, &format!("{template_id}.template")).await;
        assert_eq!(template.page_ids.len(), 1);
        assert!(copied_meta.contains(&format!(
            "_template_{template_id}/{}.page",
            template.page_ids[0]
        )));

        let metadata: proto::Metadata = read_proto(&storage, dst, "metadata/en.meta").await;
        assert_eq!(metadata.name, "Copy");
        assert_eq!(metadata.icon.as_deref(), Some("icon_id"));
        let copied_storage = keys(&storage, dst).await;
        for rel in [
            "media/icon_id.webp".to_string(),
            "upload/rpa/board_main/screenshots/a b.png".to_string(),
            "storage/db/items.lance/data/0.lance".to_string(),
            "storage/feedback.db".to_string(),
            format!("metadata/widgets/{widget_id}/en.meta"),
        ] {
            assert!(
                copied_storage.contains(&rel),
                "{rel} missing from {copied_storage:?}"
            );
        }
        let db = storage
            .get(&path(dst, "storage/db/items.lance/data/0.lance"))
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        assert_eq!(db.len(), 64 * 1024);

        assert!(report.warnings.iter().any(|w| w.contains("board_gone")));
        assert!(report.warnings.iter().any(|w| w.contains("event_orphan")));
        assert!(report.warnings.iter().any(|w| w.contains("page_missing")));
    }

    /// Tauri commands spawn their future, so it must be `Send`.
    #[test]
    fn the_duplicate_future_is_send() {
        fn assert_send<T: Send>(_: T) {}
        let (state, _, _) = setup_state();
        assert_send(App::duplicate_local(state, SRC, None));
    }

    #[tokio::test]
    async fn only_offline_apps_can_be_duplicated_and_failures_leave_nothing_behind() {
        let (state, meta, storage) = setup_state();
        seed(&meta, &storage).await;
        let mut manifest: proto::App = read_proto(&meta, SRC, "manifest.app").await;
        manifest.visibility = proto::AppVisibility::Private as i32;
        put_proto(&meta, "manifest.app", &manifest).await;

        let error = App::duplicate_local(state, SRC, None).await.unwrap_err();
        assert!(error.to_string().contains("only offline apps"));
        let apps: BTreeSet<String> = {
            let store: Arc<dyn ObjectStore> = meta.clone();
            list_prefix(&store, &Path::from("apps"))
                .await
                .unwrap()
                .into_iter()
                .map(|(rel, _)| rel.split('/').next().unwrap().to_string())
                .collect()
        };
        assert_eq!(apps, BTreeSet::from([SRC.to_string()]));
    }

    #[tokio::test]
    async fn a_board_this_build_cannot_read_fails_the_duplicate_and_is_cleaned_up() {
        let (state, meta, storage) = setup_state();
        seed(&meta, &storage).await;
        // Boards and pages are already written when the pinned version fails.
        let future = proto::Board {
            id: "board_main".into(),
            format_version: 999,
            ..Default::default()
        };
        put_proto(&meta, "versions/board_main/1_0_0.board", &future).await;

        let error = App::duplicate_local(state, SRC, None).await.unwrap_err();
        assert!(error.to_string().contains("board_main"), "{error:#}");
        for store in [&meta, &storage] {
            let store: Arc<dyn ObjectStore> = store.clone();
            let apps: BTreeSet<String> = list_prefix(&store, &Path::from("apps"))
                .await
                .unwrap()
                .into_iter()
                .map(|(rel, _)| rel.split('/').next().unwrap().to_string())
                .collect();
            assert!(apps.iter().all(|app| app == SRC), "{apps:?}");
        }
    }
}
