use crate::{
    ensure_permission, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like::{
    app::{App, AppVisibility},
    flow::{board::Board, event::Event},
};
use flow_like_types::Context;
use std::collections::{BTreeMap, BTreeSet};

macro_rules! require {
    ($condition:expr, $message:literal) => {
        if !$condition {
            return Err(ApiError::bad_request($message));
        }
    };
}

const MAX_DOCUMENTS: usize = 1024;
const MAX_BYTES: usize = 32 * 1024 * 1024;
/// The export is a GET; its address must stay well under what a proxy accepts.
const MAX_LATEST_EVENTS: usize = 64;

/// The event types a device runs, as the placements answer lists them in `event_types`. An
/// event with a default Page goes to a device whatever its type.
pub(crate) const DEVICE_EVENT_TYPES: [&str; 11] = [
    "http",
    "simple_chat",
    "rest",
    "mcp",
    "daemon",
    "cron",
    "api",
    "quick_action",
    "generic_form",
    "telegram",
    "discord",
];

/// Exported only when the request names them in `types`. The bundle carries every deployable
/// event of the app with its flow version and Pages, whatever the deploy chose: a client that
/// cannot run these types would receive them, and an app near the export limits would lose the
/// deploys it has.
const NAMED_EVENT_TYPES: [&str; 5] = ["api", "quick_action", "generic_form", "telegram", "discord"];

fn concrete(version: (u32, u32, u32)) -> bool {
    ![version.0, version.1, version.2].contains(&u32::MAX)
}

fn key(kind: &str, id: &str, version: (u32, u32, u32)) -> Result<String, ApiError> {
    flow_like_device_protocol::validate_instance_identifier(id)
        .map_err(|_| ApiError::bad_request("Invalid executable metadata identity"))?;
    if !concrete(version) {
        return Err(ApiError::bad_request(
            "Publish project dependencies before deploying them",
        ));
    }
    Ok(format!(
        "{kind}/{id}/versions/{}/{}/{}",
        version.0, version.1, version.2
    ))
}

struct Documents {
    values: BTreeMap<String, serde_json::Value>,
    bytes: usize,
}
impl Documents {
    fn add(&mut self, path: String, value: impl serde::Serialize) -> Result<(), ApiError> {
        let value = serde_json::to_value(value)?;
        let size = serde_json::to_vec(&value)?.len() + path.len();
        require!(
            self.values.len() < MAX_DOCUMENTS && self.bytes.saturating_add(size) <= MAX_BYTES,
            "Executable metadata exceeds its export limit"
        );
        require!(
            !self.values.contains_key(&path),
            "Duplicate executable metadata identity"
        );
        self.bytes += size;
        self.values.insert(path, value);
        Ok(())
    }
}

/// Matches every sink's credential field (endpoint, bot and access tokens, webhook
/// secrets, passwords, API keys) but no routing or display key. Mirrors the device-side
/// redaction in flow-like-runtime's sharing::device so both sides strip the same keys.
fn is_credential_config_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    matches!(key.as_str(), "token" | "secret" | "password")
        || key.starts_with("secret_")
        || ["_token", "_secret", "_password", "api_key"]
            .iter()
            .any(|suffix| key.ends_with(suffix))
}

/// Every event a device receives, exported or fetched by an instance, goes through here.
/// The device listener has its own secret and placements supply secrets privately, so no
/// saved sink credential (hosted endpoint token, bot token, webhook secret, mailbox
/// password) becomes a credential that outlives the device's grant.
pub(crate) fn device_event(mut event: Event) -> Result<Event, ApiError> {
    if let Ok(serde_json::Value::Object(mut config)) =
        serde_json::from_slice::<serde_json::Value>(&event.config)
    {
        let before = config.len();
        config.retain(|key, _| !is_credential_config_key(key));
        if config.len() != before {
            event.config = serde_json::to_vec(&config)?;
        }
    }
    Ok(super::events::db::filter_event_secrets(event))
}

/// The published snapshot a device receives: the working copy's version when it
/// was published, else the newest one. `published` is newest first, as
/// `App::get_widget_versions` returns it. `None` means never published.
fn deployable_widget_version(
    current: Option<(u32, u32, u32)>,
    published: &[(u32, u32, u32)],
) -> Option<(u32, u32, u32)> {
    current
        .filter(|version| published.contains(version))
        .or_else(|| published.first().copied())
}

async fn append_current_template(
    documents: &mut Documents,
    app: &App,
    id: &str,
) -> Result<(), ApiError> {
    // Templates archive the outgoing version. Their current head and its pages
    // live at unversioned paths; the controller digest freezes these exact bytes.
    let mut template = app.get_template(id, None).await?;
    require!(template.id == id, "Current template identity differs");
    let path = key("templates", id, template.version)?;
    for page_id in &template.page_ids {
        flow_like_device_protocol::validate_instance_identifier(page_id)
            .map_err(|_| ApiError::bad_request("Invalid template Page identity"))?;
        let page = template.load_template_page(id, page_id, None, None).await?;
        documents.add(format!("{path}/pages/{page_id}"), page)?;
    }
    super::board::secrets::filter_board_secrets(&mut template);
    documents.add(path, template)?;
    Ok(())
}

async fn append_widget(documents: &mut Documents, app: &App, id: &str) -> Result<(), ApiError> {
    let current = app.open_widget(id.to_string(), None).await?;
    let published = app.get_widget_versions(id).await?;
    let (version, widget) = match deployable_widget_version(current.version, &published) {
        Some(version) => (
            version,
            app.open_widget(id.to_string(), Some(version)).await?,
        ),
        // Like templates, a widget that was never published ships its working
        // copy. The controller digest freezes these exact bytes, and the
        // fallback label sits below every version a publish can allocate.
        None => {
            let mut widget = current;
            (*widget.version.get_or_insert_default(), widget)
        }
    };
    require!(
        widget.id == id && widget.version == Some(version),
        "Widget identity differs"
    );
    documents.add(key("widgets", id, version)?, widget)
}

#[derive(serde::Deserialize)]
pub struct ExportQuery {
    /// Comma-separated ids of the events of this deploy that follow Latest.
    latest: Option<String>,
    /// Comma-separated types out of `NAMED_EVENT_TYPES` whose events are exported as well.
    types: Option<String>,
}

/// The types of `NAMED_EVENT_TYPES` a client asks to have exported, each named once.
fn named_types(types: Option<&str>) -> Result<BTreeSet<String>, ApiError> {
    let mut named = BTreeSet::new();
    for name in types
        .filter(|value| !value.is_empty())
        .into_iter()
        .flat_map(|value| value.split(','))
    {
        require!(
            NAMED_EVENT_TYPES.contains(&name),
            "types lists event types out of api, quick_action, generic_form, telegram and discord"
        );
        require!(
            named.insert(name.to_owned()),
            "types names an event type twice"
        );
    }
    Ok(named)
}

/// The events a client asks to have resolved from Latest.
fn named_latest(latest: Option<&str>) -> Result<BTreeSet<String>, ApiError> {
    let ids = latest
        .filter(|value| !value.is_empty())
        .map(|value| value.split(',').collect::<Vec<_>>())
        .unwrap_or_default();
    require!(
        ids.len() <= MAX_LATEST_EVENTS,
        "One export resolves at most 64 events that follow Latest"
    );
    for id in &ids {
        flow_like_device_protocol::validate_instance_identifier(id)
            .map_err(|_| ApiError::bad_request("Invalid event identity in latest"))?;
    }
    Ok(ids.into_iter().map(str::to_owned).collect())
}

/// Where the export reads a published flow version from.
enum PublishedBoards<'a> {
    /// The hub's board cache.
    Hub {
        state: &'a AppState,
        sub: &'a str,
        app_id: &'a str,
    },
    /// The project store itself.
    #[cfg(test)]
    Stored(std::sync::Arc<flow_like::state::FlowLikeState>),
}

impl PublishedBoards<'_> {
    async fn board(
        &self,
        board_id: &str,
        version: (u32, u32, u32),
    ) -> flow_like_types::Result<Board> {
        match self {
            Self::Hub { state, sub, app_id } => {
                state
                    .master_board(sub, app_id, board_id, state, Some(version))
                    .await
            }
            #[cfg(test)]
            Self::Stored(state) => {
                let root = flow_like_storage::Path::from("apps/project");
                Board::load(root, board_id, state.clone(), Some(version)).await
            }
        }
    }
}

/// Rows 1, 3 and 4 of the event rule: active, no canary or variants, a concrete event
/// version and a kind a device runs, of a named type when it is one of `NAMED_EVENT_TYPES`.
fn deployable(event: &Event, types: &BTreeSet<String>) -> bool {
    let event_type = event.event_type.as_str();
    event.active
        && event.canary.is_none()
        && event.variants.is_empty()
        && concrete(event.event_version)
        && (event.default_page_id.is_some()
            || (DEVICE_EVENT_TYPES.contains(&event_type)
                && (!NAMED_EVENT_TYPES.contains(&event_type) || types.contains(event_type))))
}

/// The published version that holds a flow as it is stored. A flow that was never published,
/// was edited since, or cannot be read or compared has none; the last case is logged, so one
/// damaged flow never fails the export of the whole project.
async fn published_flow(app: &App, boards: &PublishedBoards<'_>, board_id: &str) -> Option<Board> {
    let published = async {
        let app_state = app.app_state.clone().context("App state not found")?;
        let store = Board::meta_store(&app_state).await?;
        let root = flow_like_storage::Path::from("apps").join(app.id.as_str());
        let Some(version) =
            Board::published_version_of_stored_draft(store, &root, board_id).await?
        else {
            return Ok(None);
        };
        let board = boards.board(board_id, version).await?;
        if board.id != board_id || board.version != version {
            flow_like_types::bail!("Published version of flow {board_id} carries another identity");
        }
        flow_like_types::Result::<Option<Board>>::Ok(Some(board))
    };
    match published.await {
        Ok(board) => board,
        Err(error) => {
            tracing::warn!(
                app_id = %app.id,
                board_id,
                error = %error,
                "Flow cannot be compared with its published versions; its events that follow Latest are left out of this export"
            );
            None
        }
    }
}

/// The flow versions that named events resolve to, read once per flow and export.
#[derive(Default)]
struct LatestFlows(BTreeMap<String, Option<Board>>);

impl LatestFlows {
    /// The version that holds the event's flow as stored, when it also holds what the event
    /// points at. Without that check one stale event makes a device refuse the whole bundle.
    async fn fitting(
        &mut self,
        app: &App,
        boards: &PublishedBoards<'_>,
        event: &Event,
    ) -> Option<&Board> {
        if !self.0.contains_key(&event.board_id) {
            let published = published_flow(app, boards, &event.board_id).await;
            self.0.insert(event.board_id.clone(), published);
        }
        self.0.get(&event.board_id)?.as_ref().filter(|board| {
            board.holds_event_target(event.default_page_id.as_deref(), &event.node_id)
        })
    }
}

async fn append_board(
    documents: &mut Documents,
    path: String,
    mut board: Board,
) -> Result<(), ApiError> {
    for page_id in &board.page_ids {
        flow_like_device_protocol::validate_instance_identifier(page_id)
            .map_err(|_| ApiError::bad_request("Invalid Page identity"))?;
        let page = board
            .load_versioned_page(page_id, board.version, None)
            .await?;
        documents.add(format!("{path}/pages/{page_id}"), page)?;
    }
    super::board::secrets::filter_board_secrets(&mut board);
    documents.add(path, board)
}

/// The event as a device receives it, or `None` when it is not exported. An event that
/// follows Latest is exported only when `latest` names it, pinned to the published version
/// that equals its flow as stored; that version comes back with it.
async fn exported_event<'flows>(
    app: &App,
    id: &str,
    latest: &BTreeSet<String>,
    types: &BTreeSet<String>,
    boards: &PublishedBoards<'_>,
    flows: &'flows mut LatestFlows,
) -> Result<Option<(Event, Option<&'flows Board>)>, ApiError> {
    let live = app.get_event(id, None).await?;
    let pinned = live.board_version.is_some_and(concrete);
    let named = live.board_version.is_none() && latest.contains(id);
    if !deployable(&live, types) || !(pinned || named) {
        return Ok(None);
    }
    let mut event = Event::load_pinned(id, app, live.event_version).await?;
    if !named || event.board_version.is_some() {
        return Ok(Some((event, None)));
    }
    let Some(board) = flows.fitting(app, boards, &event).await else {
        return Ok(None);
    };
    event.board_version = Some(board.version);
    Ok(Some((event, Some(board))))
}

/// Adds every event a device can run, with the flow version it is pinned to.
async fn append_events(
    documents: &mut Documents,
    app: &App,
    latest: &BTreeSet<String>,
    types: &BTreeSet<String>,
    boards: &PublishedBoards<'_>,
) -> Result<(), ApiError> {
    let mut flows = LatestFlows::default();
    for id in &app.events {
        let Some((event, resolved)) =
            exported_event(app, id, latest, types, boards, &mut flows).await?
        else {
            continue;
        };
        let version = event
            .board_version
            .context("Published event has no board version")?;
        let path = key("boards", &event.board_id, version)?;
        if !documents.values.contains_key(&path) {
            let board = match resolved {
                Some(board) => board.clone(),
                None => boards.board(&event.board_id, version).await?,
            };
            require!(
                board.id == event.board_id && board.version == version,
                "Published board identity differs"
            );
            append_board(documents, path, board).await?;
        }
        documents.add(
            key("events", &event.id, event.event_version)?,
            device_event(event)?,
        )?;
    }
    Ok(())
}

/// The export's operation, merged into the API's OpenAPI document.
#[derive(utoipa::OpenApi)]
#[openapi(paths(export))]
pub(crate) struct DeviceMetadataApi;

/// The client hashes and approves this content before sending it to the device.
/// The API provides no approval signature or trusted digest.
#[utoipa::path(
    get,
    path = "/apps/{app_id}/device-metadata",
    tag = "devices",
    description = "Download what a device needs to run this online project: its published flows with their Pages, its widgets and templates, and the events a device can run, each at the version it is pinned to. Saved credentials of events (endpoint tokens, bot tokens, webhook secrets, passwords, API keys) are left out; a device gets its secrets from the service's settings. Deploying to a device does this for you.",
    params(
        ("app_id" = String, Path, description = "Project ID"),
        ("latest" = Option<String>, Query, description = "Comma-separated IDs of at most 64 events of this deploy that follow the latest flow. Each is exported at the published flow version that equals its flow as saved, when there is one. An event that follows the latest flow and is not named here is left out."),
        ("types" = Option<String>, Query, description = "Comma-separated event types to export as well, each named once: `api` (Endpoints), `quick_action`, `generic_form`, `telegram` and `discord`. Events of these types are exported only when their type is named here. Without it the export holds Pages, web requests, chats, REST and MCP servers, background flows and schedules.")
    ),
    responses(
        (status = 200, description = "The project's documents for a device, keyed by their path", body = Object),
        (status = 400, description = "A parameter is not valid, the project is local-only, or it exceeds the export limits (1,024 documents, 32 MiB)"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "You cannot read this project's flows, events, templates or widgets")
    ),
    security(("bearer_auth" = []), ("pat" = []))
)]
pub async fn export(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Query(query): Query<ExportQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let required = RolePermissions::ReadBoards
        | RolePermissions::ReadEvents
        | RolePermissions::ReadTemplates
        | RolePermissions::ReadWidgets;
    let permission = ensure_permission!(user, &app_id, &state, required);
    let sub = permission.sub()?;
    let latest = named_latest(query.latest.as_deref())?;
    let types = named_types(query.types.as_deref())?;
    let mut app = state.master_app(&sub, &app_id, &state).await?;
    state.hydrate_app_visibility(&mut app).await?;
    require!(
        !matches!(app.visibility, AppVisibility::Offline),
        "Only online projects use executable metadata export"
    );
    require!(
        app.events.len() <= 512 && app.templates.len() <= 256 && app.widget_ids.len() <= 256,
        "Executable metadata inventory exceeds its limit"
    );
    let mut documents = Documents {
        values: BTreeMap::new(),
        bytes: 0,
    };

    let boards = PublishedBoards::Hub {
        state: &state,
        sub: &sub,
        app_id: &app_id,
    };
    append_events(&mut documents, &app, &latest, &types, &boards).await?;
    for id in &app.widget_ids {
        append_widget(&mut documents, &app, id).await?;
    }
    for id in &app.templates {
        append_current_template(&mut documents, &app, id).await?;
    }
    // The approved app advertises exactly the published executable inventory in this bundle.
    app.events.retain(|id| {
        documents
            .values
            .keys()
            .any(|path| path.starts_with(&format!("events/{id}/versions/")))
    });
    app.boards.retain(|id| {
        documents
            .values
            .keys()
            .any(|path| path.starts_with(&format!("boards/{id}/versions/")))
    });
    app.page_ids.retain(|id| {
        documents
            .values
            .keys()
            .any(|path| path.ends_with(&format!("/pages/{id}")))
    });
    documents.add("app".into(), &app)?;
    // Membership may change during a large export.
    crate::ensure_fresh_permission!(user, &app_id, &state, required);
    let value = serde_json::json!({"version":1,"project_id":app_id,"documents":documents.values});
    require!(
        serde_json::to_vec(&value)?.len() <= MAX_BYTES,
        "Executable metadata exceeds its export limit"
    );
    Ok(Json(value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like::state::FlowLikeState;
    use flow_like_storage::object_store::ObjectStore;
    use std::sync::Arc;

    async fn memory_project() -> (Arc<dyn ObjectStore>, Arc<FlowLikeState>, App) {
        use flow_like::{bit::Metadata, state::FlowLikeConfig, utils::http::HTTPClient};
        use flow_like_storage::{files::store::FlowLikeStore, object_store::memory::InMemory};
        let store: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
        let state = Arc::new(FlowLikeState::new(
            FlowLikeConfig::with_default_store(FlowLikeStore::Other(store.clone())),
            HTTPClient::new_without_refetch(),
        ));
        let app = App::new(
            Some("project".into()),
            Metadata::default(),
            vec![],
            state.clone(),
        )
        .await
        .unwrap();
        (store, state, app)
    }

    #[tokio::test]
    async fn widget_export_ships_published_snapshots_and_never_published_working_copies() {
        use flow_like::a2ui::widget::{VersionType, Widget};
        let (_, _, mut app) = memory_project().await;
        let mut drafted = Widget::new("drafted", "Drafted", "root");
        drafted.version = None;
        app.save_widget(&drafted).await.unwrap();
        let mut forked = Widget::new("forked", "Forked", "root");
        forked.version = Some((1, 2, 0));
        app.save_widget(&forked).await.unwrap();
        app.save_widget(&Widget::new("published", "Published", "root"))
            .await
            .unwrap();
        let version = app
            .create_widget_version("published", VersionType::Minor)
            .await
            .unwrap();
        let mut edited = app.open_widget("published".into(), None).await.unwrap();
        edited.name = "Unpublished edit".into();
        app.save_widget(&edited).await.unwrap();

        let mut documents = Documents {
            values: BTreeMap::new(),
            bytes: 0,
        };
        for id in ["drafted", "forked", "published"] {
            append_widget(&mut documents, &app, id).await.unwrap();
        }
        let drafted = &documents.values["widgets/drafted/versions/0/0/0"];
        assert_eq!(
            (drafted["id"].as_str(), drafted["version"].clone()),
            (Some("drafted"), serde_json::json!([0, 0, 0]))
        );
        assert_eq!(
            documents.values["widgets/forked/versions/1/2/0"]["name"],
            "Forked"
        );
        assert_eq!(version, (0, 1, 0));
        assert_eq!(
            documents.values["widgets/published/versions/0/1/0"]["name"],
            "Published"
        );
        assert_eq!(documents.values.len(), 3);
    }

    #[tokio::test]
    async fn template_export_freezes_current_head_and_pages_without_requiring_an_archive() {
        use flow_like::{
            a2ui::widget::Page, flow::board::Board, utils::compression::compress_to_file,
        };
        use flow_like_storage::{Path, object_store::ObjectStoreExt};
        let (store, state, app) = memory_project().await;
        let root = Path::from("apps/project");
        let mut template = Board::new(Some("template".into()), root.clone(), state);
        template.version = (3, 2, 1);
        template.page_ids = vec!["page".into()];
        template.save_as_template(None, None).await.unwrap();
        let page = Page::new("page", "Current template Page", "/").with_board_id("source-board");
        let page_path = Board::template_pages_dir(&root, "template").join("page.page");
        let proto: flow_like_types::proto::Page = page.into();
        compress_to_file(store.clone(), page_path.clone(), &proto)
            .await
            .unwrap();
        assert!(app.get_template("template", Some((3, 2, 1))).await.is_err());
        let mut documents = Documents {
            values: BTreeMap::new(),
            bytes: 0,
        };
        append_current_template(&mut documents, &app, "template")
            .await
            .unwrap();
        assert_eq!(
            documents.values["templates/template/versions/3/2/1"]["id"],
            "template"
        );
        assert_eq!(
            documents.values["templates/template/versions/3/2/1/pages/page"]["name"],
            "Current template Page"
        );
        assert!(app.get_template("template", Some((3, 2, 1))).await.is_err());
        store.delete(&page_path).await.unwrap();
        let mut incomplete = Documents {
            values: BTreeMap::new(),
            bytes: 0,
        };
        assert!(
            append_current_template(&mut incomplete, &app, "template")
                .await
                .is_err()
        );
    }

    #[test]
    fn metadata_export_omits_hosted_http_credentials_and_preserves_other_config() {
        use flow_like_types::FromProto;
        let mut event = Event::from_proto(flow_like_types::proto::Event::default());
        event.event_type = "http".into();
        event.config = br#"{"path":"/hook","method":"POST","auth_token":"cloud-endpoint-secret","settings":{"value":42}}"#.to_vec();
        let exported = device_event(event.clone()).unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&exported.config).unwrap(),
            serde_json::json!({"path":"/hook","method":"POST","settings":{"value":42}})
        );
        assert!(
            !String::from_utf8(exported.config)
                .unwrap()
                .contains("cloud-endpoint-secret")
        );
        for event_type in ["api", "webhook", "daemon"] {
            event.event_type = event_type.into();
            assert!(
                !String::from_utf8(device_event(event.clone()).unwrap().config)
                    .unwrap()
                    .contains("cloud-endpoint-secret"),
                "{event_type} kept its hosted token"
            );
        }
        for (event_type, config, kept) in [
            (
                "telegram",
                serde_json::json!({"bot_token":"bot-credential","webhook_secret":"hook-credential","chat_id":"42"}),
                serde_json::json!({"chat_id":"42"}),
            ),
            (
                "user_mail",
                serde_json::json!({"imap_host":"mail.example","secret_imap_password":"mailbox-credential"}),
                serde_json::json!({"imap_host":"mail.example"}),
            ),
            (
                "http",
                serde_json::json!({"path":"/hook","AUTH_TOKEN":"cloud-endpoint-secret","Api_Key":"key-credential"}),
                serde_json::json!({"path":"/hook"}),
            ),
        ] {
            event.event_type = event_type.into();
            event.config = serde_json::to_vec(&config).unwrap();
            let exported = device_event(event.clone()).unwrap().config;
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&exported).unwrap(),
                kept,
                "{event_type} kept a sink credential"
            );
        }
        for config in [
            br#"{"path":"/hook","settings":{"auth_token":"nested-flow-value"}}"#.to_vec(),
            br#"{"path":"/hook","tokenizer":"words","secretary":"desk"}"#.to_vec(),
            b"not json".to_vec(),
            Vec::new(),
        ] {
            event.config = config;
            assert_eq!(device_event(event.clone()).unwrap().config, event.config);
        }
    }

    #[test]
    fn deployable_widget_version_prefers_the_published_working_copy_version() {
        let published = [(1, 2, 0), (1, 1, 0)];
        assert_eq!(
            deployable_widget_version(Some((1, 1, 0)), &published),
            Some((1, 1, 0))
        );
        assert_eq!(deployable_widget_version(None, &published), Some((1, 2, 0)));
        assert_eq!(
            deployable_widget_version(Some((2, 0, 0)), &published),
            Some((1, 2, 0))
        );
        assert_eq!(deployable_widget_version(Some((0, 0, 1)), &[]), None);
        assert_eq!(deployable_widget_version(None, &[]), None);
    }

    #[test]
    fn metadata_export_rejects_ambiguous_or_unbounded_documents() {
        let mut documents = Documents {
            values: BTreeMap::new(),
            bytes: 0,
        };
        documents
            .add("app".into(), serde_json::json!({"id":"project"}))
            .unwrap();
        assert!(
            documents
                .add("app".into(), serde_json::json!({"id":"other"}))
                .is_err()
        );
        documents.bytes = MAX_BYTES;
        assert!(
            documents
                .add("other".into(), serde_json::json!({}))
                .is_err()
        );
        assert!(key("boards", "../other", (1, 0, 0)).is_err());
        assert!(key("boards", "board", (u32::MAX, 0, 0)).is_err());
    }

    /// A stored flow of the project with one Page (`page`) and one start node, whose id is
    /// returned.
    async fn stored_flow(state: &Arc<FlowLikeState>, id: &str) -> (Board, String) {
        use flow_like::{a2ui::widget::Page, flow::node::Node};
        let root = flow_like_storage::Path::from("apps/project");
        let mut board = Board::new(Some(id.into()), root, state.clone());
        let node = Node::new("start", "Start", "", "test");
        let node_id = node.id.clone();
        board.nodes.insert(node_id.clone(), node);
        board
            .save_page(&Page::new("page", "Start", "/"), None)
            .await
            .unwrap();
        board.save(None).await.unwrap();
        (board, node_id)
    }

    /// Saves an active event of the project that points at `node_id` of `board_id`.
    async fn stored_event(
        app: &mut App,
        id: &str,
        event_type: &str,
        board_id: &str,
        node_id: &str,
        board_version: Option<(u32, u32, u32)>,
    ) -> Event {
        use flow_like_types::FromProto;
        let mut event = Event::from_proto(flow_like_types::proto::Event::default());
        event.id = id.into();
        event.name = id.into();
        event.event_type = event_type.into();
        event.board_id = board_id.into();
        event.node_id = node_id.into();
        event.board_version = board_version;
        event.event_version = (1, 0, 0);
        event.active = true;
        event.save(app, None).await.unwrap();
        app.events.push(id.into());
        event
    }

    async fn exported(
        app: &App,
        state: &Arc<FlowLikeState>,
        latest: &[&str],
    ) -> BTreeMap<String, serde_json::Value> {
        exported_with(app, state, latest, &[]).await
    }

    async fn exported_with(
        app: &App,
        state: &Arc<FlowLikeState>,
        latest: &[&str],
        types: &[&str],
    ) -> BTreeMap<String, serde_json::Value> {
        let mut documents = Documents {
            values: BTreeMap::new(),
            bytes: 0,
        };
        let named = latest.iter().map(|id| (*id).to_owned()).collect();
        let types = types.iter().map(|name| (*name).to_owned()).collect();
        let boards = PublishedBoards::Stored(state.clone());
        append_events(&mut documents, app, &named, &types, &boards)
            .await
            .unwrap();
        documents.values
    }

    /// Ids of the exported events, sorted.
    fn exported_events(documents: &BTreeMap<String, serde_json::Value>) -> Vec<&str> {
        let mut ids = documents
            .keys()
            .filter_map(|path| path.strip_prefix("events/")?.split('/').next())
            .collect::<Vec<_>>();
        ids.sort_unstable();
        ids
    }

    /// The rule the export applied before it could resolve Latest or ship a schedule.
    fn exported_before(event: &Event) -> bool {
        event.active
            && event.canary.is_none()
            && event.variants.is_empty()
            && event.board_version.is_some_and(concrete)
            && concrete(event.event_version)
            && (event.default_page_id.is_some()
                || matches!(
                    event.event_type.as_str(),
                    "http" | "simple_chat" | "rest" | "mcp" | "daemon"
                ))
    }

    #[tokio::test]
    async fn pinned_schedules_are_exported_and_a_schedule_with_a_page_stays_as_it_was() {
        let (_, state, mut app) = memory_project().await;
        let (mut flow, node) = stored_flow(&state, "flow").await;
        let (version, _) = flow.publish_if_changed(None).await.unwrap();
        let pinned = Some(version);

        stored_event(&mut app, "schedule", "cron", "flow", &node, pinned).await;
        let mut with_page =
            stored_event(&mut app, "schedule-page", "cron", "flow", &node, pinned).await;
        with_page.default_page_id = Some("page".into());
        with_page.save(&app, None).await.unwrap();
        let mut paused = stored_event(&mut app, "paused", "cron", "flow", &node, pinned).await;
        paused.active = false;
        paused.save(&app, None).await.unwrap();
        stored_event(&mut app, "bot", "telegram", "flow", &node, pinned).await;

        let documents = exported(&app, &state, &[]).await;
        assert_eq!(exported_events(&documents), ["schedule", "schedule-page"]);
        assert_eq!(
            documents["events/schedule/versions/1/0/0"]["event_type"],
            "cron"
        );
        assert_eq!(
            documents["events/schedule-page/versions/1/0/0"]["default_page_id"],
            "page"
        );
        assert!(documents.contains_key("boards/flow/versions/0/0/1"));
        assert!(documents.contains_key("boards/flow/versions/0/0/1/pages/page"));
    }

    #[tokio::test]
    async fn without_named_events_the_bundle_is_what_it_was_before_latest_could_be_resolved() {
        let (_, state, mut app) = memory_project().await;
        let (mut flow, node) = stored_flow(&state, "flow").await;
        let (version, _) = flow.publish_if_changed(None).await.unwrap();
        let pinned = Some(version);

        let mut events = Vec::new();
        for (id, event_type, board_version) in [
            ("pinned-http", "http", pinned),
            ("pinned-daemon", "daemon", pinned),
            ("pinned-bot", "telegram", pinned),
            (
                "pinned-sentinel",
                "http",
                Some((u32::MAX, u32::MAX, u32::MAX)),
            ),
            ("latest-http", "http", None),
            ("latest-daemon", "daemon", None),
            ("latest-schedule", "cron", None),
        ] {
            events.push(stored_event(&mut app, id, event_type, "flow", &node, board_version).await);
        }
        let mut paused = stored_event(&mut app, "paused", "http", "flow", &node, pinned).await;
        paused.active = false;
        paused.save(&app, None).await.unwrap();
        events.push(paused);

        let documents = exported(&app, &state, &[]).await;
        let mut before = events
            .iter()
            .filter(|event| exported_before(event))
            .map(|event| event.id.as_str())
            .collect::<Vec<_>>();
        before.sort_unstable();
        assert_eq!(before, ["pinned-daemon", "pinned-http"]);
        assert_eq!(exported_events(&documents), before);
        assert_eq!(
            documents.keys().collect::<Vec<_>>(),
            [
                "boards/flow/versions/0/0/1",
                "boards/flow/versions/0/0/1/pages/page",
                "events/pinned-daemon/versions/1/0/0",
                "events/pinned-http/versions/1/0/0",
            ]
        );
        let stored = Event::load_pinned("pinned-http", &app, (1, 0, 0))
            .await
            .unwrap();
        assert_eq!(
            documents["events/pinned-http/versions/1/0/0"],
            serde_json::to_value(device_event(stored).unwrap()).unwrap()
        );
    }

    #[tokio::test]
    async fn a_named_latest_event_is_exported_at_the_version_that_equals_its_flow() {
        let (_, state, mut app) = memory_project().await;
        let (mut flow, node) = stored_flow(&state, "flow").await;
        let (version, _) = flow.publish_if_changed(None).await.unwrap();
        stored_event(&mut app, "named", "http", "flow", &node, None).await;
        stored_event(&mut app, "named-schedule", "cron", "flow", &node, None).await;
        stored_event(&mut app, "not-named", "http", "flow", &node, None).await;
        stored_event(&mut app, "pinned", "http", "flow", &node, Some(version)).await;

        let documents = exported(
            &app,
            &state,
            &["named", "named-schedule", "pinned", "unknown"],
        )
        .await;
        assert_eq!(
            exported_events(&documents),
            ["named", "named-schedule", "pinned"],
            "a Latest event that is not named stays out; a named pinned or unknown one changes nothing"
        );
        for id in ["named", "named-schedule"] {
            assert_eq!(
                documents[&format!("events/{id}/versions/1/0/0")]["board_version"],
                serde_json::json!([0, 0, 1]),
                "the shipped copy of {id} carries the resolved version"
            );
        }
        assert_eq!(documents["boards/flow/versions/0/0/1"]["id"], "flow");
        assert!(documents.contains_key("boards/flow/versions/0/0/1/pages/page"));
        assert_eq!(
            app.get_event("named", None).await.unwrap().board_version,
            None,
            "the event record keeps following Latest"
        );
        assert_eq!(flow.get_versions(None).await.unwrap(), vec![version]);
    }

    #[tokio::test]
    async fn named_latest_events_without_a_version_that_equals_their_flow_are_left_out() {
        let (_, state, mut app) = memory_project().await;
        let (_, never_node) = stored_flow(&state, "never-published").await;
        let (mut edited, edited_node) = stored_flow(&state, "edited-since").await;
        let (version, _) = edited.publish_if_changed(None).await.unwrap();
        edited.name = "Edited after its newest version".into();
        edited.mark_changed();
        edited.save(None).await.unwrap();

        stored_event(
            &mut app,
            "never",
            "http",
            "never-published",
            &never_node,
            None,
        )
        .await;
        stored_event(
            &mut app,
            "edited",
            "http",
            "edited-since",
            &edited_node,
            None,
        )
        .await;
        stored_event(&mut app, "no-flow", "http", "missing-flow", "node", None).await;
        stored_event(
            &mut app,
            "sentinel",
            "http",
            "edited-since",
            &edited_node,
            Some((u32::MAX, 0, 0)),
        )
        .await;
        stored_event(
            &mut app,
            "pinned",
            "http",
            "edited-since",
            &edited_node,
            Some(version),
        )
        .await;

        let named = exported(&app, &state, &["never", "edited", "no-flow", "sentinel"]).await;
        assert_eq!(
            exported_events(&named),
            ["pinned"],
            "a version that differs from the stored flow is never shipped for a Latest event"
        );
        assert_eq!(named, exported(&app, &state, &[]).await);
    }

    #[tokio::test]
    async fn a_named_event_that_lost_its_target_is_left_out_and_the_rest_is_unchanged() {
        let (_, state, mut app) = memory_project().await;
        let (mut flow, node) = stored_flow(&state, "flow").await;
        flow.publish_if_changed(None).await.unwrap();
        stored_event(&mut app, "fits", "http", "flow", &node, None).await;
        let mut page_fits =
            stored_event(&mut app, "page-fits", "cron", "flow", "removed-node", None).await;
        page_fits.default_page_id = Some("page".into());
        page_fits.save(&app, None).await.unwrap();

        let fitting = exported(&app, &state, &["fits", "page-fits"]).await;
        assert_eq!(exported_events(&fitting), ["fits", "page-fits"]);

        stored_event(&mut app, "node-gone", "http", "flow", "removed-node", None).await;
        let mut page_gone = stored_event(&mut app, "page-gone", "http", "flow", &node, None).await;
        page_gone.default_page_id = Some("removed-page".into());
        page_gone.save(&app, None).await.unwrap();

        assert_eq!(
            exported(
                &app,
                &state,
                &["fits", "page-fits", "node-gone", "page-gone"]
            )
            .await,
            fitting,
            "one stale event must not change, or break, the bundle the others are deployed from"
        );
    }

    #[test]
    fn named_events_are_bounded_valid_identifiers() {
        assert!(named_latest(None).unwrap().is_empty());
        assert!(named_latest(Some("")).unwrap().is_empty());
        assert_eq!(
            named_latest(Some("evt_b,evt_a,evt_b")).unwrap(),
            BTreeSet::from(["evt_a".to_owned(), "evt_b".to_owned()])
        );
        let ids = |count: usize| {
            (0..count)
                .map(|index| format!("event-{index}"))
                .collect::<Vec<_>>()
                .join(",")
        };
        assert_eq!(named_latest(Some(&ids(64))).unwrap().len(), 64);
        for refused in [
            ids(65),
            "evt_a,,evt_b".into(),
            "evt_a,../other".into(),
            "evt a".into(),
        ] {
            let error = named_latest(Some(&refused)).unwrap_err();
            assert_eq!(
                error.status(),
                axum::http::StatusCode::BAD_REQUEST,
                "{refused}"
            );
        }
    }

    #[test]
    fn named_types_are_the_five_new_ones_each_once() {
        assert!(named_types(None).unwrap().is_empty());
        assert!(named_types(Some("")).unwrap().is_empty());
        assert_eq!(
            named_types(Some("telegram,api")).unwrap(),
            BTreeSet::from(["api".to_owned(), "telegram".to_owned()])
        );
        assert_eq!(
            named_types(Some("api,discord,generic_form,quick_action,telegram")).unwrap(),
            NAMED_EVENT_TYPES.map(str::to_owned).into_iter().collect()
        );
        for refused in [
            "webhook",
            "cron",
            "http",
            "API",
            " api",
            "api,api",
            "api,,telegram",
            ",api",
            "api,",
        ] {
            let error = named_types(Some(refused)).unwrap_err();
            assert_eq!(
                error.status(),
                axum::http::StatusCode::BAD_REQUEST,
                "{refused}"
            );
        }
    }

    #[test]
    fn the_device_types_are_round_ones_and_the_five_named_ones() {
        assert_eq!(
            DEVICE_EVENT_TYPES,
            [
                "http",
                "simple_chat",
                "rest",
                "mcp",
                "daemon",
                "cron",
                "api",
                "quick_action",
                "generic_form",
                "telegram",
                "discord"
            ]
        );
        assert_eq!(
            DEVICE_EVENT_TYPES
                .into_iter()
                .filter(|event_type| !NAMED_EVENT_TYPES.contains(event_type))
                .collect::<Vec<_>>(),
            ["http", "simple_chat", "rest", "mcp", "daemon", "cron"],
            "without `types` the export admits exactly round one's types"
        );
        assert!(
            NAMED_EVENT_TYPES
                .iter()
                .all(|event_type| DEVICE_EVENT_TYPES.contains(event_type))
        );
    }

    /// The config a device receives for the exported event `id`.
    fn exported_config(
        documents: &BTreeMap<String, serde_json::Value>,
        id: &str,
    ) -> serde_json::Value {
        let event: Event =
            serde_json::from_value(documents[&format!("events/{id}/versions/1/0/0")].clone())
                .unwrap();
        serde_json::from_slice(&event.config).unwrap()
    }

    #[tokio::test]
    async fn a_new_type_is_exported_only_when_named_and_without_its_credentials() {
        let (_, state, mut app) = memory_project().await;
        let (mut flow, node) = stored_flow(&state, "flow").await;
        let (version, _) = flow.publish_if_changed(None).await.unwrap();
        let configs = [
            (
                "api",
                serde_json::json!({"sink_type":"http","method":"GET","path":"/orders","public_endpoint":false}),
            ),
            (
                "quick_action",
                serde_json::json!({"label":"Restart the line"}),
            ),
            (
                "generic_form",
                serde_json::json!({"fields":[{"name":"order","type":"String"}]}),
            ),
            (
                "telegram",
                serde_json::json!({"chat_whitelist":["42"],"respond_to_mentions":true}),
            ),
            ("discord", serde_json::json!({"command_prefix":"!"})),
        ];
        let credentials = serde_json::json!({
            "auth_token": "endpoint-credential",
            "token": "plain-credential",
            "bot_token": "bot-credential",
            "webhook_secret": "hook-credential"
        });
        for (event_type, config) in &configs {
            let mut event = stored_event(
                &mut app,
                event_type,
                event_type,
                "flow",
                &node,
                Some(version),
            )
            .await;
            let mut stored = config.clone();
            stored
                .as_object_mut()
                .unwrap()
                .extend(credentials.as_object().unwrap().clone());
            event.config = serde_json::to_vec(&stored).unwrap();
            event.save(&app, None).await.unwrap();
        }
        stored_event(
            &mut app,
            "mailbox",
            "user_mail",
            "flow",
            &node,
            Some(version),
        )
        .await;
        stored_event(&mut app, "endpoint", "http", "flow", &node, Some(version)).await;

        let all = exported_with(&app, &state, &[], &NAMED_EVENT_TYPES).await;
        assert_eq!(
            exported_events(&all),
            [
                "api",
                "discord",
                "endpoint",
                "generic_form",
                "quick_action",
                "telegram"
            ],
            "a type a device does not run stays out whatever is named"
        );
        for (event_type, config) in &configs {
            assert_eq!(
                all[&format!("events/{event_type}/versions/1/0/0")]["event_type"],
                *event_type
            );
            assert_eq!(
                exported_config(&all, event_type),
                *config,
                "{event_type} keeps its settings and loses every credential"
            );
        }

        let some = exported_with(&app, &state, &[], &["telegram", "api"]).await;
        assert_eq!(
            exported_events(&some),
            ["api", "endpoint", "telegram"],
            "a new type that is not named stays out"
        );
        assert_eq!(
            exported(&app, &state, &[]).await.keys().collect::<Vec<_>>(),
            [
                "boards/flow/versions/0/0/1",
                "boards/flow/versions/0/0/1/pages/page",
                "events/endpoint/versions/1/0/0",
            ]
        );
    }

    /// The export of every app stays round one's for a client that names no type: the events
    /// of the five types leave no document, no flow version and no Page behind.
    #[tokio::test]
    async fn without_types_the_bundle_is_round_ones_for_an_app_with_every_new_type() {
        let (_, state, mut app) = memory_project().await;
        let (mut flow, node) = stored_flow(&state, "flow").await;
        let (version, _) = flow.publish_if_changed(None).await.unwrap();
        let (mut bot_flow, bot_node) = stored_flow(&state, "bot-flow").await;
        let (bot_version, _) = bot_flow.publish_if_changed(None).await.unwrap();

        for event_type in DEVICE_EVENT_TYPES {
            stored_event(
                &mut app,
                &format!("pinned-{event_type}"),
                event_type,
                "flow",
                &node,
                Some(version),
            )
            .await;
        }
        for event_type in NAMED_EVENT_TYPES {
            stored_event(
                &mut app,
                &format!("only-{event_type}"),
                event_type,
                "bot-flow",
                &bot_node,
                Some(bot_version),
            )
            .await;
            stored_event(
                &mut app,
                &format!("latest-{event_type}"),
                event_type,
                "bot-flow",
                &bot_node,
                None,
            )
            .await;
        }
        let mut page_bot = stored_event(
            &mut app,
            "page-telegram",
            "telegram",
            "flow",
            &node,
            Some(version),
        )
        .await;
        page_bot.default_page_id = Some("page".into());
        page_bot.save(&app, None).await.unwrap();
        stored_event(&mut app, "latest-http", "http", "flow", &node, None).await;
        let latest = ["latest-http", "latest-api", "latest-telegram"];

        let bundle = exported(&app, &state, &latest).await;
        assert_eq!(
            exported_events(&bundle),
            [
                "latest-http",
                "page-telegram",
                "pinned-cron",
                "pinned-daemon",
                "pinned-http",
                "pinned-mcp",
                "pinned-rest",
                "pinned-simple_chat"
            ]
        );
        assert!(
            !bundle
                .keys()
                .any(|path| path.starts_with("boards/bot-flow/")),
            "a flow only the five types use is not exported"
        );

        let new_types = |id: &String| {
            NAMED_EVENT_TYPES
                .iter()
                .any(|event_type| id.ends_with(&format!("-{event_type}")))
                && id != "page-telegram"
        };
        app.events.retain(|id| !new_types(id));
        assert_eq!(
            serde_json::to_vec(&bundle).unwrap(),
            serde_json::to_vec(&exported(&app, &state, &latest).await).unwrap(),
            "the bundle equals the one of the same app without those events, byte for byte"
        );
    }

    #[test]
    fn the_export_documents_both_parameters_for_the_apps_author() {
        use utoipa::OpenApi;
        let spec = serde_json::to_value(crate::openapi::ApiDoc::openapi()).unwrap();
        let operation = &spec["paths"]["/apps/{app_id}/device-metadata"]["get"];
        assert_eq!(operation["tags"], serde_json::json!(["devices"]));
        assert!(operation["security"].is_array());
        let parameters = operation["parameters"].as_array().expect("parameters");
        for (name, mentions) in [("latest", "64"), ("types", "telegram")] {
            let parameter = parameters
                .iter()
                .find(|parameter| parameter["name"] == name)
                .unwrap_or_else(|| panic!("{name} is documented"));
            assert_eq!(parameter["in"], "query", "{name}");
            assert_eq!(parameter["required"], false, "{name}");
            assert!(
                parameter["description"]
                    .as_str()
                    .is_some_and(|text| text.contains(mentions)),
                "{name}"
            );
        }
    }
}
