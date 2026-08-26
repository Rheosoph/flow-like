use crate::{
    a2ui::widget::Page,
    bit::Metadata,
    flow::{
        board::{Board, VersionType, commands::nodes::copy_paste::CopyPasteCommand},
        event::Event,
    },
    state::FlowLikeState,
    utils::compression::{
        compress_to_file, compress_to_file_json, from_compressed, from_compressed_json,
    },
};
use flow_like_storage::Path;
use flow_like_types::{FromProto, ToProto, create_id, proto, sync::Mutex};
use futures::{StreamExt, TryStreamExt};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc, time::SystemTime, vec};
pub mod sharing;

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub enum StandardInterfaces {
    Chat,
    Search,
    Form,
    List,
    A2UI,
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct FrontendConfiguration {
    pub landing_page: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub enum AppCategory {
    Other = 0,
    Productivity = 1,
    Social = 2,
    Entertainment = 3,
    Education = 4,
    Health = 5,
    Finance = 6,
    Lifestyle = 7,
    Travel = 8,
    News = 9,
    Sports = 10,
    Shopping = 11,
    FoodAndDrink = 12,
    Music = 13,
    Photography = 14,
    Utilities = 15,
    Weather = 16,
    Games = 17,
    Business = 18,
    Communication = 19,
    Anime = 20,
}

/// What kind of thing the app is, structurally — an agent, a pipeline, a form.
/// Orthogonal to [`AppCategory`], which says what the app is *about*. Left
/// unset until the owner classifies it; the UI derives a suggestion from the
/// app's contents in the meantime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum AppType {
    Agent = 0,
    CustomInterface = 1,
    DataFocus = 2,
    DataPipeline = 3,
    Analytics = 4,
    Form = 5,
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub enum AppStatus {
    Active = 0,
    Inactive = 1,
    Archived = 2,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub enum AppVisibility {
    Public = 0,
    PublicRequestAccess = 1,
    Private = 2,
    Prototype = 3,
    Offline = 4,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub enum AppExecutionMode {
    Any = 0,
    Local = 1,
    Remote = 2,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, Clone)]
pub enum AppSearchSort {
    BestRated,
    WorstRated,
    MostPopular,
    LeastPopular,
    MostRelevant,
    LeastRelevant,
    NewestCreated,
    OldestCreated,
    NewestUpdated,
    OldestUpdated,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, Clone)]
pub struct AppSearchQuery {
    pub id: Option<String>,
    pub query: Option<String>,
    pub language: Option<String>,
    pub limit: Option<u64>,
    pub offset: Option<u64>,
    pub category: Option<AppCategory>,
    pub author: Option<String>,
    pub sort: Option<AppSearchSort>,
    pub tag: Option<String>,
}

/// What [`App::create_board`] produced.
///
/// A board instantiated from a template writes its pages to storage as a side effect, and those
/// pages are the only ones no page-upload call ever describes. Deployments that keep a page row
/// per page therefore need them handed back, or the pages exist on disk and nowhere else.
pub struct CreatedBoard {
    pub board_id: String,
    pub pages: Vec<Page>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct App {
    pub id: String,

    pub status: AppStatus,
    pub visibility: AppVisibility,

    pub authors: Vec<String>,
    pub bits: Vec<String>,
    pub boards: Vec<String>,
    pub events: Vec<String>,
    pub templates: Vec<String>,

    pub changelog: Option<String>,

    pub primary_category: Option<AppCategory>,
    pub secondary_category: Option<AppCategory>,

    /// Owner-declared app type. `None` means unclassified.
    #[serde(default)]
    pub app_type: Option<AppType>,

    pub rating_sum: u64,
    pub rating_count: u64,
    pub download_count: u64,
    pub interactions_count: u64,

    pub avg_rating: Option<f64>,
    pub relevance_score: Option<f64>,
    pub execution_mode: AppExecutionMode,

    pub updated_at: SystemTime,
    pub created_at: SystemTime,

    pub version: Option<String>,

    pub frontend: Option<FrontendConfiguration>,

    pub price: Option<u32>,

    // A2UI Integration - stored as IDs, loaded separately
    #[serde(default)]
    pub widget_ids: Vec<String>,
    #[serde(default)]
    pub page_ids: Vec<String>,

    /// WASM packages required by this app: package_id -> version
    #[serde(default)]
    pub packages: HashMap<String, String>,

    /// Project-level opt-in for the Fork-an-app feature. Apps default to
    /// `false`; the owner must explicitly allow forking before any other
    /// permission check applies.
    #[serde(default)]
    pub allow_forking: bool,

    /// For forked apps: the source app's id (for lineage / attribution).
    #[serde(default)]
    pub forked_from: Option<String>,

    /// For forked apps: when the fork was created.
    #[serde(default)]
    pub forked_at: Option<SystemTime>,

    #[serde(skip)]
    pub app_state: Option<Arc<FlowLikeState>>,
}

impl Clone for App {
    fn clone(&self) -> Self {
        Self {
            id: self.id.clone(),
            status: self.status.clone(),
            visibility: self.visibility.clone(),
            authors: self.authors.clone(),
            boards: self.boards.clone(),
            templates: self.templates.clone(),
            bits: self.bits.clone(),
            events: self.events.clone(),
            changelog: self.changelog.clone(),
            avg_rating: self.avg_rating,
            download_count: self.download_count,
            interactions_count: self.interactions_count,
            rating_count: self.rating_count,
            rating_sum: self.rating_sum,
            relevance_score: self.relevance_score,
            primary_category: self.primary_category.clone(),
            secondary_category: self.secondary_category.clone(),
            app_type: self.app_type.clone(),
            updated_at: self.updated_at,
            created_at: self.created_at,
            version: self.version.clone(),
            price: self.price,
            execution_mode: self.execution_mode.clone(),
            app_state: self.app_state.clone(),
            frontend: self.frontend.clone(),
            widget_ids: self.widget_ids.clone(),
            page_ids: self.page_ids.clone(),
            packages: self.packages.clone(),
            allow_forking: self.allow_forking,
            forked_from: self.forked_from.clone(),
            forked_at: self.forked_at,
        }
    }
}

impl App {
    pub async fn new(
        id: Option<String>,
        meta: Metadata,
        bits: Vec<String>,
        app_state: Arc<FlowLikeState>,
    ) -> flow_like_types::Result<Self> {
        let id = id.unwrap_or(create_id());

        App::push_meta(id.clone(), meta, app_state.clone(), None, None).await?;

        let item = Self {
            id,
            authors: vec![],
            bits,
            boards: vec![],
            events: vec![],
            templates: vec![],
            updated_at: SystemTime::now(),
            created_at: SystemTime::now(),
            version: None,
            status: AppStatus::Active,
            visibility: AppVisibility::Offline,
            changelog: None,
            avg_rating: None,
            download_count: 0,
            interactions_count: 0,
            rating_count: 0,
            rating_sum: 0,
            relevance_score: None,
            execution_mode: AppExecutionMode::Any,

            primary_category: None,
            secondary_category: None,
            app_type: None,

            price: None,

            frontend: None,
            widget_ids: vec![],
            page_ids: vec![],
            packages: HashMap::new(),
            allow_forking: false,
            forked_from: None,
            forked_at: None,
            app_state: Some(app_state.clone()),
        };

        Ok(item)
    }

    pub async fn load(id: String, app_state: Arc<FlowLikeState>) -> flow_like_types::Result<Self> {
        let storage_root = Path::from("apps").child(id.clone());

        let store = FlowLikeState::project_meta_store(&app_state)
            .await?
            .as_generic();

        let app: flow_like_types::proto::App =
            from_compressed(store, storage_root.child("manifest.app")).await?;
        let mut app = App::from_proto(app);
        app.app_state = Some(app_state.clone());

        Ok(app)
    }

    pub fn calculate_relevance_score(&mut self) -> f64 {
        let downloads = self.download_count as f64;
        let sum_ratings = self.rating_sum as f64;
        let rating_count = self.rating_count as f64;
        let interactions = self.interactions_count as f64;
        let avg_rating = sum_ratings / rating_count;
        self.avg_rating = Some(avg_rating);
        let relevance =
            (downloads * 2.0 + interactions) * (1.0 + avg_rating / 5.0) * (rating_count.ln() + 1.0);
        self.relevance_score = Some(relevance);
        relevance
    }

    pub async fn get_meta(
        id: String,
        app_state: Arc<FlowLikeState>,
        language: Option<String>,
        template_id: Option<String>,
    ) -> flow_like_types::Result<Metadata> {
        let store = FlowLikeState::project_storage_store(&app_state)
            .await?
            .as_generic();

        let mut metadata_path = Path::from("apps").child(id).child("metadata");
        if let Some(template_id) = template_id {
            metadata_path = metadata_path.child("templates").child(template_id);
        }
        let languages = [
            language.unwrap_or_else(|| "en".to_string()),
            "en".to_string(),
        ];

        // Try requested language first, then fallback to English
        for lang in languages
            .iter()
            .take_while(|&l| l != &languages[1] || l == &languages[0])
        {
            let meta_path = metadata_path.child(format!("{}.meta", lang));

            if let Ok(metadata) = from_compressed::<proto::Metadata>(store.clone(), meta_path).await
            {
                return Ok(Metadata::from_proto(metadata));
            }
        }

        Err(flow_like_types::anyhow!(
            "No metadata found for app {}",
            metadata_path
        ))
    }

    pub async fn push_meta(
        id: String,
        metadata: Metadata,
        app_state: Arc<FlowLikeState>,
        language: Option<String>,
        template_id: Option<String>,
    ) -> flow_like_types::Result<()> {
        let store = FlowLikeState::project_storage_store(&app_state)
            .await?
            .as_generic();

        let language = language.unwrap_or_else(|| "en".to_string());
        let mut meta_path = Path::from("apps").child(id).child("metadata");

        if let Some(template_id) = template_id {
            meta_path = meta_path.child("templates").child(template_id);
        }

        let meta_path = meta_path.child(format!("{}.meta", language));

        let proto_metadata = metadata.to_proto();
        compress_to_file(store, meta_path, &proto_metadata).await?;

        Ok(())
    }

    pub async fn create_board(
        &mut self,
        id: Option<String>,
        template: Option<Board>,
    ) -> flow_like_types::Result<CreatedBoard> {
        let storage_root = Path::from("apps").child(self.id.clone());
        let state = self
            .app_state
            .clone()
            .ok_or(flow_like_types::anyhow!("App state not found"))?;
        let mut board = Board::new(id, storage_root.clone(), state.clone());
        let mut pages = Vec::new();
        if let Some(mut template) = template {
            // A template that crossed a serialization boundary — an API request body, the desktop
            // IPC bridge — arrives without `app_state`/`board_dir`, which are runtime-only. Its
            // page payloads still live on this app's store under the template layout, so rebind it
            // before trying to read them.
            if template.app_state.is_none() {
                template.app_state = Some(state.clone());
                template.board_dir = storage_root.clone();
            }
            board.variables = template.variables.clone();
            let mut paste_command = {
                let nodes = template.nodes.values().cloned().collect::<Vec<_>>();
                let comments = template.comments.values().cloned().collect::<Vec<_>>();
                let layers = template.layers.values().cloned().collect::<Vec<_>>();
                CopyPasteCommand::new(nodes, comments, layers, (0.0, 0.0, 0.0))
            };
            // Schemas and descriptions are compacted into `Board::refs`. They must be present
            // while the paste command runs because `execute_command` performs node migrations and
            // cleanup immediately afterwards. Adding them later makes cleanup hash the ref key as
            // if it were the schema itself, leaving the instantiated board with an unresolved
            // ref-to-ref chain.
            paste_command.original_variables = template.variables.values().cloned().collect();
            paste_command.original_refs = template
                .refs
                .iter()
                .filter(|(key, _)| !crate::flow::board::is_internal_board_ref(key.as_str()))
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect();
            let paste_command =
                crate::flow::board::commands::GenericCommand::CopyPaste(paste_command);
            let executed = board.execute_command(paste_command, state.clone()).await?;
            // Page payloads point at nodes by id, so they can only follow the copy if they know
            // what the paste renamed each node to.
            let node_translation = match executed {
                crate::flow::board::commands::GenericCommand::CopyPaste(command) => {
                    command.translated_ids
                }
                _ => HashMap::new(),
            };

            let source_app_id = template.board_dir.filename().map(str::to_string);
            let app_translation = source_app_id
                .as_deref()
                .filter(|source_app_id| *source_app_id != self.id.as_str())
                .map(|source_app_id| (source_app_id, self.id.as_str()));

            pages = board
                .instantiate_template_pages(&template, &node_translation, app_translation, None)
                .await?;

            // A board payload replayed under its own id — the offline-to-online migration ships
            // one per board and uploads that board's pages separately — keeps the page ids it came
            // with, because those uploads are what materializes them. Ids belonging to a *different*
            // board are never adopted: `Page.id` is a global primary key, so a second board
            // claiming them collides with the original the moment either is persisted.
            if pages.is_empty() && template.id == board.id {
                board.page_ids = template.page_ids.clone();
                board.mark_changed();
            }
        }
        board.save(None).await?;
        self.boards.push(board.id.clone());
        self.updated_at = SystemTime::now();
        Ok(CreatedBoard {
            board_id: board.id,
            pages,
        })
    }

    pub async fn boards_configured(&self) -> bool {
        for board_id in &self.boards {
            let board = self.open_board(board_id.clone(), Some(false), None).await;
            if let Ok(board) = board {
                let vars = board
                    .lock()
                    .await
                    .variables
                    .values()
                    .cloned()
                    .collect::<Vec<_>>();
                for var in vars {
                    if var.default_value.is_none() {
                        return false;
                    }
                }
            }
        }

        true
    }

    pub async fn open_board(
        &self,
        board_id: String,
        register: Option<bool>,
        version: Option<(u32, u32, u32)>,
    ) -> flow_like_types::Result<Arc<Mutex<Board>>> {
        let storage_root = Path::from("apps").child(self.id.clone());
        if let Some(app_state) = &self.app_state {
            let board = app_state.get_board(&board_id, version);

            if let Ok(board) = board {
                return Ok(board);
            }
        }

        let state = self
            .app_state
            .clone()
            .ok_or(flow_like_types::anyhow!("App state not found"))?;

        let board = Board::load(storage_root, &board_id, state, version).await?;
        let board_ref = Arc::new(Mutex::new(board));
        let register = register.unwrap_or(false);
        if register && let Some(app_state) = &self.app_state {
            app_state.register_board(&board_id, board_ref.clone(), version)?;
        }

        Ok(board_ref)
    }

    /// Load a board directly from its persisted object, bypassing the
    /// process-local board registry.
    ///
    /// The registry is ideal for interactive editing, but publication paths
    /// must not use a cached draft as their source of truth: another API
    /// process may already have committed a newer floating board under the
    /// same semantic version. Immutable/version-pinning workflows should use
    /// this method before deciding which content to publish.
    pub async fn open_board_authoritative(
        &self,
        board_id: String,
        version: Option<(u32, u32, u32)>,
    ) -> flow_like_types::Result<Arc<Mutex<Board>>> {
        let storage_root = Path::from("apps").child(self.id.clone());
        let state = self
            .app_state
            .clone()
            .ok_or(flow_like_types::anyhow!("App state not found"))?;
        let board = Board::load(storage_root, &board_id, state, version).await?;
        Ok(Arc::new(Mutex::new(board)))
    }

    pub async fn delete_board(&mut self, board_id: &str) -> flow_like_types::Result<()> {
        self.boards.retain(|b| b != board_id);
        let board_dir = Path::from("apps")
            .child(self.id.clone())
            .child(format!("{}.board", board_id));

        let state = self
            .app_state
            .clone()
            .ok_or(flow_like_types::anyhow!("App state not found"))?;
        let store = FlowLikeState::project_meta_store(&state)
            .await?
            .as_generic();
        store.delete(&board_dir).await?;

        if let Some(app_state) = &self.app_state {
            app_state.remove_board(board_id)?;
        }

        // Remove all versions of the board
        let versions_path = Path::from("apps")
            .child(self.id.clone())
            .child("versions")
            .child(board_id);
        let locations = store
            .list(Some(&versions_path))
            .map_ok(|m| m.location)
            .boxed();

        store
            .delete_stream(locations)
            .try_collect::<Vec<Path>>()
            .await?;

        // Compiled artifacts are derived from the versions removed above.
        let compiled_path = Path::from("apps")
            .child(self.id.clone())
            .child("compiled")
            .child(board_id);
        let compiled_locations = store
            .list(Some(&compiled_path))
            .map_ok(|m| m.location)
            .boxed();
        store
            .delete_stream(compiled_locations)
            .try_collect::<Vec<Path>>()
            .await?;

        self.updated_at = SystemTime::now();
        self.save().await?;
        Ok(())
    }

    // EVENTS

    pub async fn get_event_versions(
        &self,
        event_id: &str,
    ) -> flow_like_types::Result<Vec<(u32, u32, u32)>> {
        let event = Event::load(event_id, self, None).await?;
        let versions = event.get_versions(self).await?;
        Ok(versions)
    }

    pub async fn get_event(
        &self,
        event_id: &str,
        version: Option<(u32, u32, u32)>,
    ) -> flow_like_types::Result<Event> {
        let event = Event::load(event_id, self, version).await?;
        Ok(event)
    }

    pub async fn upsert_event(
        &mut self,
        event: Event,
        version_type: Option<VersionType>,
        enforce_id: Option<bool>,
    ) -> flow_like_types::Result<Event> {
        let enforce_id = enforce_id.unwrap_or(false);
        let mut event = event;

        let saved_event = event.upsert(self, version_type, enforce_id).await?;

        if !self.events.contains(&saved_event.id) {
            self.events.push(saved_event.id.clone());
        }

        self.updated_at = SystemTime::now();
        self.save().await?;
        Ok(saved_event)
    }

    pub async fn validate_event(
        &self,
        event_id: &str,
        version: Option<(u32, u32, u32)>,
    ) -> flow_like_types::Result<()> {
        let event = Event::load(event_id, self, version).await?;
        event.validate_event_references(self).await?;

        Ok(())
    }

    pub async fn delete_event(&mut self, event_id: &str) -> flow_like_types::Result<()> {
        self.events.retain(|e| e != event_id);

        let event = Event::load(event_id, self, None).await?;
        event.delete(self).await?;

        self.updated_at = SystemTime::now();
        self.save().await?;
        Ok(())
    }

    // TEMPLATES

    pub async fn upsert_template(
        &mut self,
        template_id: Option<String>,
        version_type: VersionType,
        board_id: String,
        board_version: Option<(u32, u32, u32)>,
    ) -> flow_like_types::Result<(String, (u32, u32, u32))> {
        let explicit_template_id = template_id.is_some();
        let mut template_id = template_id.unwrap_or_else(create_id);
        let new_template: Arc<Mutex<Board>> = self
            .open_board(board_id, Some(false), board_version)
            .await?;
        let old_template = self.open_template(template_id.clone(), None).await.ok();

        if old_template.is_none() && !explicit_template_id {
            template_id = create_id();
        }

        let template: (u32, u32, u32) = new_template
            .lock()
            .await
            .create_template(template_id.clone(), version_type, old_template, None)
            .await?;

        if !self.templates.contains(&template_id) {
            self.templates.push(template_id.clone());
        }

        self.updated_at = SystemTime::now();
        self.save().await?;

        Ok((template_id, template))
    }

    pub async fn push_template_data(
        &self,
        template_id: String,
        data: Board,
        version: Option<(u32, u32, u32)>,
    ) -> flow_like_types::Result<()> {
        let app_state = self
            .app_state
            .clone()
            .ok_or(flow_like_types::anyhow!("App state not found"))?;
        let mut data = data;
        data.app_state = Some(app_state.clone());
        data.id = template_id.clone();
        data.board_dir = Path::from("apps").child(self.id.clone());

        // Record-only: this is a template fetched from the hub being cached locally, so its page
        // ids have no payloads on this store to copy from.
        if let Some(version) = version {
            data.overwrite_template_version(version, None, None).await?;
        } else {
            data.save_as_template(None, None).await?;
        }

        Ok(())
    }

    pub async fn get_template(
        &self,
        template_id: &str,
        version: Option<(u32, u32, u32)>,
    ) -> flow_like_types::Result<Board> {
        let storage_root = Path::from("apps").child(self.id.clone());

        let state = self
            .app_state
            .clone()
            .ok_or(flow_like_types::anyhow!("App state not found"))?;

        let template = Board::load_template(storage_root, template_id, state, version).await?;

        Ok(template)
    }

    pub async fn get_template_versions(
        &self,
        template_id: &str,
    ) -> flow_like_types::Result<Vec<(u32, u32, u32)>> {
        let storage_root = Path::from("apps").child(self.id.clone());

        let state = self
            .app_state
            .clone()
            .ok_or(flow_like_types::anyhow!("App state not found"))?;

        let template = Board::load_template(storage_root, template_id, state, None).await?;
        let versions = template.get_template_versions(None).await?;
        Ok(versions)
    }

    pub async fn open_template(
        &self,
        template_id: String,
        version: Option<(u32, u32, u32)>,
    ) -> flow_like_types::Result<Board> {
        let storage_root = Path::from("apps").child(self.id.clone());

        let state = self
            .app_state
            .clone()
            .ok_or(flow_like_types::anyhow!("App state not found"))?;

        let template = Board::load_template(storage_root, &template_id, state, version).await?;

        Ok(template)
    }

    pub async fn delete_template(&mut self, template_id: &str) -> flow_like_types::Result<()> {
        self.templates.retain(|b| b != template_id);
        let template_dir = Path::from("apps")
            .child(self.id.clone())
            .child(format!("{}.template", template_id));

        let state = self
            .app_state
            .clone()
            .ok_or(flow_like_types::anyhow!("App state not found"))?;
        let store = FlowLikeState::project_meta_store(&state)
            .await?
            .as_generic();
        store.delete(&template_dir).await?;

        // Remove all versions of the board
        let versions_path = Path::from("apps")
            .child(self.id.clone())
            .child("templates")
            .child("versions")
            .child(template_id);
        let locations = store
            .list(Some(&versions_path))
            .map_ok(|m| m.location)
            .boxed();

        store
            .delete_stream(locations)
            .try_collect::<Vec<Path>>()
            .await?;

        // The template's own page payloads sit next to the board files, outside the version tree
        // the sweep above walks, so they would otherwise outlive the template forever.
        let pages_path =
            Board::template_pages_dir(&Path::from("apps").child(self.id.clone()), template_id);
        let page_locations = store.list(Some(&pages_path)).map_ok(|m| m.location).boxed();
        // A template that never had pages has no directory at all, which a
        // filesystem store reports as an error rather than an empty listing.
        // Leaving payloads behind is a leak; failing the delete over their
        // absence would be a bug.
        if let Err(error) = store
            .delete_stream(page_locations)
            .try_collect::<Vec<Path>>()
            .await
        {
            tracing::warn!("sweeping pages of template {}: {}", template_id, error);
        }

        self.updated_at = SystemTime::now();
        self.save().await?;
        Ok(())
    }

    pub async fn push_template_meta(
        &self,
        template_id: &str,
        language: Option<String>,
        meta: Metadata,
    ) -> flow_like_types::Result<()> {
        let language = language.unwrap_or_else(|| "en".to_string());
        let store = FlowLikeState::project_storage_store(&self.app_state.clone().unwrap())
            .await?
            .as_generic();

        let meta_path = Path::from("apps")
            .child(self.id.clone())
            .child("metadata")
            .child("templates")
            .child(template_id)
            .child(format!("{}.meta", language));

        let proto_metadata = meta.to_proto();
        compress_to_file(store, meta_path, &proto_metadata).await?;
        Ok(())
    }

    pub async fn get_template_meta(
        &self,
        template_id: &str,
        language: Option<String>,
    ) -> flow_like_types::Result<Metadata> {
        let store = FlowLikeState::project_storage_store(&self.app_state.clone().unwrap())
            .await?
            .as_generic();

        let language = language.unwrap_or_else(|| "en".to_string());
        let meta_path = Path::from("apps")
            .child(self.id.clone())
            .child("metadata")
            .child("templates")
            .child(template_id)
            .child(format!("{}.meta", language));

        let metadata = from_compressed::<proto::Metadata>(store.clone(), meta_path).await;
        if let Err(e) = metadata {
            eprintln!("Failed to get template metadata: {}", e);
            let meta_path = Path::from("apps")
                .child(self.id.clone())
                .child("metadata")
                .child("templates")
                .child(template_id)
                .child("en.meta");
            let metadata = from_compressed::<proto::Metadata>(store, meta_path).await;
            if let Err(e) = metadata {
                eprintln!("Failed to get template metadata in English: {}", e);
                return Err(flow_like_types::anyhow!(
                    "No metadata found for template {} in any language",
                    template_id
                ));
            }
            return Ok(Metadata::from_proto(metadata?));
        }

        Ok(Metadata::from_proto(metadata?))
    }

    // WIDGETS

    /// Get all widgets for this app
    pub async fn get_widgets(&self) -> flow_like_types::Result<Vec<crate::a2ui::widget::Widget>> {
        let mut widgets = Vec::with_capacity(self.widget_ids.len());
        for widget_id in &self.widget_ids {
            if let Ok(widget) = self.open_widget(widget_id.clone(), None).await {
                widgets.push(widget);
            }
        }
        Ok(widgets)
    }

    /// Open/load a widget by ID with optional version
    pub async fn open_widget(
        &self,
        widget_id: String,
        version: Option<(u32, u32, u32)>,
    ) -> flow_like_types::Result<crate::a2ui::widget::Widget> {
        let state = self
            .app_state
            .clone()
            .ok_or(flow_like_types::anyhow!("App state not found"))?;
        let store = FlowLikeState::project_meta_store(&state)
            .await?
            .as_generic();

        let widget_path = if let Some(v) = version {
            Path::from("apps")
                .child(self.id.clone())
                .child("widgets")
                .child("versions")
                .child(widget_id.as_str())
                .child(format!("{}-{}-{}.widget", v.0, v.1, v.2))
        } else {
            Path::from("apps")
                .child(self.id.clone())
                .child(format!("{}.widget", widget_id))
        };

        let widget: crate::a2ui::widget::Widget = from_compressed_json(store, widget_path).await?;
        Ok(widget)
    }

    /// Save/create a widget
    pub async fn save_widget(
        &mut self,
        widget: &crate::a2ui::widget::Widget,
    ) -> flow_like_types::Result<()> {
        let state = self
            .app_state
            .clone()
            .ok_or(flow_like_types::anyhow!("App state not found"))?;
        let store = FlowLikeState::project_meta_store(&state)
            .await?
            .as_generic();

        let widget_path = Path::from("apps")
            .child(self.id.clone())
            .child(format!("{}.widget", widget.id));

        compress_to_file_json(store, widget_path, widget).await?;

        // Seed the metadata sidecar exactly like the remote widget upsert does.
        // Local-only creation paths never wrote one, so every listing that reads
        // names from metadata fell back to showing the raw widget id.
        if !self.has_widget_meta(&widget.id).await {
            let meta = Metadata {
                name: widget.name.clone(),
                description: widget.description.clone().unwrap_or_default(),
                tags: widget.tags.clone(),
                ..Default::default()
            };
            if let Err(e) = self.push_widget_meta(&widget.id, None, meta).await {
                eprintln!("Failed to seed metadata for widget {}: {}", widget.id, e);
            }
        }

        // Add widget ID to the list if not already present
        if !self.widget_ids.contains(&widget.id) {
            self.widget_ids.push(widget.id.clone());
            self.save().await?;
        }

        Ok(())
    }

    /// Quiet existence check for a widget's metadata sidecar.
    async fn has_widget_meta(&self, widget_id: &str) -> bool {
        let Some(state) = self.app_state.clone() else {
            return false;
        };
        let Ok(store) = FlowLikeState::project_storage_store(&state).await else {
            return false;
        };

        let meta_path = Path::from("apps")
            .child(self.id.clone())
            .child("metadata")
            .child("widgets")
            .child(widget_id)
            .child("en.meta");

        store.as_generic().head(&meta_path).await.is_ok()
    }

    /// Delete a widget
    pub async fn delete_widget(&mut self, widget_id: &str) -> flow_like_types::Result<()> {
        let state = self
            .app_state
            .clone()
            .ok_or(flow_like_types::anyhow!("App state not found"))?;
        let store = FlowLikeState::project_meta_store(&state)
            .await?
            .as_generic();

        let widget_path = Path::from("apps")
            .child(self.id.clone())
            .child(format!("{}.widget", widget_id));
        store.delete(&widget_path).await?;

        // Delete all versions
        let versions_path = Path::from("apps")
            .child(self.id.clone())
            .child("widgets")
            .child("versions")
            .child(widget_id);
        let locations = store
            .list(Some(&versions_path))
            .map_ok(|m| m.location)
            .boxed();
        store
            .delete_stream(locations)
            .try_collect::<Vec<Path>>()
            .await?;

        // Remove widget ID from the list
        self.widget_ids.retain(|id| id != widget_id);
        self.save().await?;

        Ok(())
    }

    /// Get all versions of a widget
    pub async fn get_widget_versions(
        &self,
        widget_id: &str,
    ) -> flow_like_types::Result<Vec<(u32, u32, u32)>> {
        let state = self
            .app_state
            .clone()
            .ok_or(flow_like_types::anyhow!("App state not found"))?;
        let store = FlowLikeState::project_meta_store(&state)
            .await?
            .as_generic();

        let versions_path = Path::from("apps")
            .child(self.id.clone())
            .child("widgets")
            .child("versions")
            .child(widget_id);

        let mut versions = Vec::new();
        let mut stream = store.list(Some(&versions_path));
        while let Some(entry) = stream.next().await {
            if let std::result::Result::Ok(entry) = entry {
                let filename = entry.location.filename().unwrap_or_default();
                if let Some(version_str) = filename.strip_suffix(".widget") {
                    let parts: Vec<&str> = version_str.split('-').collect();
                    if parts.len() == 3
                        && let (
                            std::result::Result::Ok(major),
                            std::result::Result::Ok(minor),
                            std::result::Result::Ok(patch),
                        ) = (parts[0].parse(), parts[1].parse(), parts[2].parse())
                    {
                        versions.push((major, minor, patch));
                    }
                }
            }
        }

        versions.sort_unstable_by(|a, b| b.cmp(a));
        Ok(versions)
    }

    /// Push widget metadata for a specific language
    pub async fn push_widget_meta(
        &self,
        widget_id: &str,
        language: Option<String>,
        meta: Metadata,
    ) -> flow_like_types::Result<()> {
        let language = language.unwrap_or_else(|| "en".to_string());
        let store = FlowLikeState::project_storage_store(&self.app_state.clone().unwrap())
            .await?
            .as_generic();

        let meta_path = Path::from("apps")
            .child(self.id.clone())
            .child("metadata")
            .child("widgets")
            .child(widget_id)
            .child(format!("{}.meta", language));

        let proto_metadata = meta.to_proto();
        compress_to_file(store, meta_path, &proto_metadata).await?;
        Ok(())
    }

    /// Get widget metadata for a specific language
    pub async fn get_widget_meta(
        &self,
        widget_id: &str,
        language: Option<String>,
    ) -> flow_like_types::Result<Metadata> {
        let store = FlowLikeState::project_storage_store(&self.app_state.clone().unwrap())
            .await?
            .as_generic();

        let language = language.unwrap_or_else(|| "en".to_string());
        let meta_path = Path::from("apps")
            .child(self.id.clone())
            .child("metadata")
            .child("widgets")
            .child(widget_id)
            .child(format!("{}.meta", language));

        let metadata = from_compressed::<proto::Metadata>(store.clone(), meta_path).await;
        if let Err(e) = metadata {
            eprintln!("Failed to get widget metadata: {}", e);
            let meta_path = Path::from("apps")
                .child(self.id.clone())
                .child("metadata")
                .child("widgets")
                .child(widget_id)
                .child("en.meta");
            let metadata = from_compressed::<proto::Metadata>(store, meta_path).await;
            if let Err(e) = metadata {
                eprintln!("Failed to get widget metadata in English: {}", e);
                return Err(flow_like_types::anyhow!(
                    "No metadata found for widget {} in any language",
                    widget_id
                ));
            }
            return Ok(Metadata::from_proto(metadata?));
        }

        Ok(Metadata::from_proto(metadata?))
    }

    // PAGES
    //
    // Page content lives on `Board` (`apps/{app}/_{board_id}/{page_id}.page`,
    // compressed binary `proto::Page`). The previous app-level helpers
    // (`save_page` / `open_page` / `get_pages` / `delete_page` /
    // `get_page_versions`) wrote a parallel JSON layout at
    // `apps/{app}/{page_id}.page` and were the source of the online-fork
    // 404s. They've been removed; route page operations through the
    // owning `Board` (`Board::save_page` / `load_page` / `delete_page` /
    // `load_versioned_page`). `Board::load_page` falls back to the
    // legacy app-level path so pages saved before this change stay
    // readable.

    /// Push page metadata for a specific language
    pub async fn push_page_meta(
        &self,
        page_id: &str,
        language: Option<String>,
        meta: Metadata,
    ) -> flow_like_types::Result<()> {
        let language = language.unwrap_or_else(|| "en".to_string());
        let store = FlowLikeState::project_storage_store(&self.app_state.clone().unwrap())
            .await?
            .as_generic();

        let meta_path = Path::from("apps")
            .child(self.id.clone())
            .child("metadata")
            .child("pages")
            .child(page_id)
            .child(format!("{}.meta", language));

        let proto_metadata = meta.to_proto();
        compress_to_file(store, meta_path, &proto_metadata).await?;
        Ok(())
    }

    /// Get page metadata for a specific language
    pub async fn get_page_meta(
        &self,
        page_id: &str,
        language: Option<String>,
    ) -> flow_like_types::Result<Metadata> {
        let store = FlowLikeState::project_storage_store(&self.app_state.clone().unwrap())
            .await?
            .as_generic();

        let language = language.unwrap_or_else(|| "en".to_string());
        let meta_path = Path::from("apps")
            .child(self.id.clone())
            .child("metadata")
            .child("pages")
            .child(page_id)
            .child(format!("{}.meta", language));

        let metadata = from_compressed::<proto::Metadata>(store.clone(), meta_path).await;
        if let Err(e) = metadata {
            eprintln!("Failed to get page metadata: {}", e);
            let meta_path = Path::from("apps")
                .child(self.id.clone())
                .child("metadata")
                .child("pages")
                .child(page_id)
                .child("en.meta");
            let metadata = from_compressed::<proto::Metadata>(store, meta_path).await;
            if let Err(e) = metadata {
                eprintln!("Failed to get page metadata in English: {}", e);
                return Err(flow_like_types::anyhow!(
                    "No metadata found for page {} in any language",
                    page_id
                ));
            }
            return Ok(Metadata::from_proto(metadata?));
        }

        Ok(Metadata::from_proto(metadata?))
    }

    pub async fn save(&self) -> flow_like_types::Result<()> {
        if let Some(app_state) = &self.app_state {
            let store = FlowLikeState::project_meta_store(app_state)
                .await?
                .as_generic();

            let board_refs = {
                let mut refs = Vec::with_capacity(self.boards.len());

                for board_id in &self.boards {
                    if let Ok(board) = app_state.get_board(board_id, None) {
                        refs.push(board.clone());
                    }
                }
                refs
            };

            for board in board_refs {
                let tmp = board.lock().await.clone();
                tmp.save(Some(store.clone())).await?;
            }
        }

        let store = self
            .app_state
            .clone()
            .ok_or(flow_like_types::anyhow!("App state not found"))?;
        let store = FlowLikeState::project_meta_store(&store)
            .await?
            .as_generic();

        let manifest_path = Path::from("apps")
            .child(self.id.clone())
            .child("manifest.app");

        let mut proto_app = self.to_proto();
        let mut seen = std::collections::HashSet::with_capacity(self.boards.len());
        proto_app.boards.retain(|b| seen.insert(b.clone()));
        compress_to_file(store, manifest_path, &proto_app).await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        bit::Metadata,
        flow::{board::Board, node::Node, variable::VariableType},
        state::FlowLikeConfig,
        utils::http::HTTPClient,
    };
    use flow_like_storage::{Path, files::store::FlowLikeStore, object_store};
    use flow_like_types::{FromProto, ToProto};
    use flow_like_types::{Message, tokio};
    use std::sync::Arc;

    async fn flow_state() -> Arc<crate::state::FlowLikeState> {
        let mut config: FlowLikeConfig = FlowLikeConfig::new();
        config.register_app_meta_store(FlowLikeStore::Other(Arc::new(
            flow_like_storage::object_store::memory::InMemory::new(),
        )));
        let http_client = HTTPClient::new_without_refetch();
        let flow_like_state = crate::state::FlowLikeState::new(config, http_client);
        Arc::new(flow_like_state)
    }

    #[tokio::test]
    async fn serialize_app() {
        let app = crate::app::App {
            id: "id".to_string(),
            app_type: None,
            authors: vec!["author1".to_string(), "author2".to_string()],
            boards: vec!["board1".to_string(), "board2".to_string()],
            bits: vec!["bit1".to_string(), "bit2".to_string()],
            events: vec!["release1".to_string(), "release2".to_string()],
            templates: vec!["template1".to_string(), "template2".to_string()],
            updated_at: std::time::SystemTime::now(),
            created_at: std::time::SystemTime::now(),
            status: crate::app::AppStatus::Active,
            visibility: crate::app::AppVisibility::Public,
            changelog: Some("Changelog text".to_string()),
            primary_category: Some(crate::app::AppCategory::Productivity),
            secondary_category: Some(crate::app::AppCategory::Education),
            app_state: Some(flow_state().await),
            version: Some("1.0.0".to_string()),
            avg_rating: Some(4.5),
            execution_mode: crate::app::AppExecutionMode::Any,
            download_count: 1000,
            interactions_count: 500,
            price: Some(9),
            rating_count: 200,
            rating_sum: 800,
            relevance_score: Some(0.9),
            frontend: None,
            widget_ids: vec![],
            page_ids: vec![],
            packages: std::collections::HashMap::new(),
            allow_forking: false,
            forked_from: None,
            forked_at: None,
        };

        let mut buf = Vec::new();
        app.to_proto().encode(&mut buf).unwrap();
        let deser = super::App::from_proto(flow_like_types::proto::App::decode(&buf[..]).unwrap());

        assert_eq!(app.id, deser.id);
    }

    #[tokio::test]
    async fn create_board_from_template_keeps_schema_refs_resolvable() {
        let store = FlowLikeStore::Other(Arc::new(object_store::memory::InMemory::new()));
        let state = Arc::new(crate::state::FlowLikeState::new(
            FlowLikeConfig::with_default_store(store),
            HTTPClient::new_without_refetch(),
        ));
        let mut app = super::App::new(
            Some("template-ref-test-app".to_string()),
            Metadata::default(),
            Vec::new(),
            state.clone(),
        )
        .await
        .expect("test app should be created");

        let schema = r#"{"type":"object","properties":{"value":{"type":"string"}}}"#;
        let schema_ref = "template-schema-ref";
        let mut template = Board::new(
            Some("source-template".to_string()),
            Path::from("apps/source"),
            state.clone(),
        );
        template
            .refs
            .insert(schema_ref.to_string(), schema.to_string());
        let mut node = Node::new("template_test_node", "Template Test", "", "Tests");
        node.add_output_pin("value", "Value", "", VariableType::Struct)
            .schema = Some(schema_ref.to_string());
        template.nodes.insert(node.id.clone(), node);

        let created = app
            .create_board(Some("instantiated-board".to_string()), Some(template))
            .await
            .expect("template should instantiate");
        let board = Board::load(
            Path::from("apps").child(app.id.clone()),
            &created.board_id,
            state,
            None,
        )
        .await
        .expect("instantiated board should load");
        let pin = board
            .nodes
            .values()
            .next()
            .and_then(|node| node.get_pin_by_name("value"))
            .expect("template pin should exist");
        let stored_ref = pin
            .schema
            .as_deref()
            .expect("template pin should stay typed");

        assert_eq!(
            board.refs.get(stored_ref).map(String::as_str),
            Some(schema),
            "the instantiated pin must point directly to its copied schema"
        );
    }

    async fn page_test_app(id: &str) -> (super::App, Arc<crate::state::FlowLikeState>) {
        let store = FlowLikeStore::Other(Arc::new(object_store::memory::InMemory::new()));
        let state = Arc::new(crate::state::FlowLikeState::new(
            FlowLikeConfig::with_default_store(store),
            HTTPClient::new_without_refetch(),
        ));
        let app = super::App::new(
            Some(id.to_string()),
            Metadata::default(),
            Vec::new(),
            state.clone(),
        )
        .await
        .expect("test app should be created");
        (app, state)
    }

    /// The offline-to-online migration re-creates each board from its own payload and uploads that
    /// board's pages afterwards, so the ids have to survive the round trip for the uploads to land
    /// on the right board.
    #[tokio::test]
    async fn create_board_from_own_payload_keeps_page_ids() {
        let (mut app, state) = page_test_app("own-payload-page-test-app").await;
        let mut payload =
            Board::new_detached(Some("instantiated-board".to_string()), Path::from("unused"));
        payload.page_ids = vec!["page-one".to_string(), "page-two".to_string()];

        let created = app
            .create_board(Some("instantiated-board".to_string()), Some(payload))
            .await
            .expect("board payload should instantiate");
        let board = Board::load(
            Path::from("apps").child(app.id.clone()),
            &created.board_id,
            state,
            None,
        )
        .await
        .expect("instantiated board should load");

        assert!(created.pages.is_empty());
        assert_eq!(board.page_ids, vec!["page-one", "page-two"]);
    }

    /// `Page.id` is a global primary key, so page ids that belong to a different board must never
    /// be adopted — they have no payload and no row behind them, and claiming them collides with
    /// the board that owns them.
    #[tokio::test]
    async fn create_board_from_template_drops_foreign_page_ids() {
        let (mut app, state) = page_test_app("detached-template-page-test-app").await;
        let mut template =
            Board::new_detached(Some("detached-template".to_string()), Path::from("unused"));
        template.page_ids = vec!["page-one".to_string(), "page-two".to_string()];

        let created = app
            .create_board(Some("instantiated-board".to_string()), Some(template))
            .await
            .expect("detached template should instantiate");
        let board = Board::load(
            Path::from("apps").child(app.id.clone()),
            &created.board_id,
            state,
            None,
        )
        .await
        .expect("instantiated board should load");

        assert!(created.pages.is_empty());
        assert!(board.page_ids.is_empty());
    }
}
