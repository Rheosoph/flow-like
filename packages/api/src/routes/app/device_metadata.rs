use crate::{
    ensure_permission, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like::{
    app::{App, AppVisibility},
    flow::event::Event,
};
use flow_like_types::Context;
use std::collections::BTreeMap;

macro_rules! require {
    ($condition:expr, $message:literal) => {
        if !$condition {
            return Err(ApiError::bad_request($message));
        }
    };
}

const MAX_DOCUMENTS: usize = 1024;
const MAX_BYTES: usize = 32 * 1024 * 1024;

fn key(kind: &str, id: &str, version: (u32, u32, u32)) -> Result<String, ApiError> {
    flow_like_device_protocol::validate_instance_identifier(id)
        .map_err(|_| ApiError::bad_request("Invalid executable metadata identity"))?;
    if [version.0, version.1, version.2].contains(&u32::MAX) {
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

/// The client hashes and approves this content before sending it to the device.
/// The API provides no approval signature or trusted digest.
pub async fn export(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let required = RolePermissions::ReadBoards
        | RolePermissions::ReadEvents
        | RolePermissions::ReadTemplates
        | RolePermissions::ReadWidgets;
    let permission = ensure_permission!(user, &app_id, &state, required);
    let sub = permission.sub()?;
    let mut app = state.master_app(&sub, &app_id, &state).await?;
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

    for id in &app.events {
        let event = app.get_event(id, None).await?;
        if !event.active
            || event.canary.is_some()
            || !event.variants.is_empty()
            || event.board_version.is_none()
            || [
                event.event_version.0,
                event.event_version.1,
                event.event_version.2,
            ]
            .contains(&u32::MAX)
            || event
                .board_version
                .is_some_and(|v| [v.0, v.1, v.2].contains(&u32::MAX))
            || !(event.default_page_id.is_some()
                || matches!(
                    event.event_type.as_str(),
                    "http" | "simple_chat" | "rest" | "mcp" | "daemon"
                ))
        {
            continue;
        }
        let event = Event::load_pinned(id, &app, event.event_version).await?;
        let version = event
            .board_version
            .context("Published event has no board version")?;
        let path = key("boards", &event.board_id, version)?;
        if !documents.values.contains_key(&path) {
            let mut board = state
                .master_board(&sub, &app_id, &event.board_id, &state, Some(version))
                .await?;
            require!(
                board.id == event.board_id && board.version == version,
                "Published board identity differs"
            );
            for page_id in &board.page_ids {
                flow_like_device_protocol::validate_instance_identifier(page_id)
                    .map_err(|_| ApiError::bad_request("Invalid Page identity"))?;
                let page = board.load_versioned_page(page_id, version, None).await?;
                documents.add(format!("{path}/pages/{page_id}"), page)?;
            }
            super::board::secrets::filter_board_secrets(&mut board);
            documents.add(path, board)?;
        }
        documents.add(
            key("events", &event.id, event.event_version)?,
            device_event(event)?,
        )?;
    }
    for id in &app.widget_ids {
        let current = app.open_widget(id.clone(), None).await?;
        let version = current
            .version
            .context("Publish widgets before deploying them")?;
        let widget = app.open_widget(id.clone(), Some(version)).await?;
        require!(
            widget.id == *id && widget.version == Some(version),
            "Published widget identity differs"
        );
        documents.add(key("widgets", id, version)?, widget)?;
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

    #[tokio::test]
    async fn template_export_freezes_current_head_and_pages_without_requiring_an_archive() {
        use flow_like::{
            a2ui::widget::Page,
            bit::Metadata,
            flow::board::Board,
            state::{FlowLikeConfig, FlowLikeState},
            utils::{compression::compress_to_file, http::HTTPClient},
        };
        use flow_like_storage::{
            Path,
            files::store::FlowLikeStore,
            object_store::{ObjectStore, ObjectStoreExt, memory::InMemory},
        };
        use std::sync::Arc;
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
        let root = Path::from("apps/project");
        let mut template = Board::new(Some("template".into()), root.clone(), state);
        template.version = (3, 2, 1);
        template.page_ids = vec!["page".into()];
        template.save_as_template(None, None).await.unwrap();
        let page = Page::new("page", "Current template Page", "/").with_board_id("source-board");
        let page_path = Board::template_pages_dir(&root, "template").child("page.page");
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
}
