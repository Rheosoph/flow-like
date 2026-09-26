use anyhow::{Context, Result, ensure};
use flow_like_runtime::{
    app::{App, AppVisibility},
    flow::{board::Board, event::Event},
    state::{FlowLikeConfig, FlowLikeState},
    utils::http::HTTPClient,
};
use flow_like_storage::files::store::{FlowLikeStore, local_store::LocalObjectStore};
use flow_like_types::FromProto;
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path, sync::Arc};

// A page leaves room for the management envelope inside a Noise message.
const PAGE_BYTES: usize = 10 * 1024;

pub async fn describe(
    state: &Path,
    project: &str,
    revision: &str,
    event_id: Option<&str>,
    after: Option<&str>,
) -> Result<Value> {
    let root = crate::project_artifacts::managed_revision(state, project, revision)?;
    if let Some(id) = event_id {
        crate::config::validate_id("event", id)?;
    }
    if let Some(id) = after {
        crate::config::validate_id("cursor", id)?;
    }
    describe_snapshot(&root, project, revision, event_id, after).await
}

async fn describe_snapshot(
    root: &Path,
    project: &str,
    revision: &str,
    event_id: Option<&str>,
    after: Option<&str>,
) -> Result<Value> {
    // Discovery needs metadata only. It never initializes node libraries, databases,
    // dependency downloads, or workflow execution in the management process.
    let mut config = FlowLikeConfig::new();
    let metadata =
        FlowLikeStore::Local(Arc::new(LocalObjectStore::new(root.to_path_buf())?)).read_only();
    config.register_app_meta_store(metadata.clone());
    let state = Arc::new(FlowLikeState::new(
        config,
        HTTPClient::new_without_refetch(),
    ));
    let app = App::load(project.to_owned(), state).await?;
    ensure!(
        app.id == project && matches!(app.visibility, AppVisibility::Offline),
        "Discovery requires the matching offline snapshot"
    );
    ensure!(
        app.events.len() <= 512,
        "Project event inventory exceeds discovery limit"
    );
    let mut rows = BTreeMap::new();
    if let Some(id) = event_id {
        ensure!(
            app.events.iter().any(|value| value == id),
            "Event is outside the project inventory"
        );
        let event = pinned_event(&app, id).await?;
        crate::config::validate_id("board", &event.board_id)?;
        let version = event
            .board_version
            .context("Event must pin a board version")?;
        ensure!(concrete(version), "Board version must be concrete");
        let board = Board::from_proto(
            Board::load_proto(
                metadata.as_generic(),
                &flow_like_storage::Path::from("apps").join(project),
                &event.board_id,
                Some(version),
            )
            .await?,
        );
        ensure!(
            board.id == event.board_id && board.version == version,
            "Board pin differs"
        );
        for variable in board.variables.values().chain(
            board
                .layers
                .values()
                .flat_map(|layer| layer.variables.values()),
        ) {
            if !variable.exposed && !variable.runtime_configured {
                continue;
            }
            crate::config::validate_id("variable", &variable.id)?;
            // Never serialize defaults, schemas, or event overrides. They may contain credentials.
            let row = json!({"id": variable.id, "name": variable.name.chars().take(120).collect::<String>(), "data_type": variable.data_type, "value_type": variable.value_type, "secret": variable.secret});
            if let Some(old) = rows.insert(variable.id.clone(), row.clone()) {
                ensure!(old == row, "Conflicting variable definitions");
            }
            ensure!(rows.len() <= 1024, "Too many configurable variables");
        }
    } else {
        let mut ids = app.events.clone();
        ids.sort();
        ids.dedup();
        // Read a bounded number of events per request, including unsupported entries.
        for id in ids
            .iter()
            .filter(|id| after.is_none_or(|cursor| id.as_str() > cursor))
            .take(8)
        {
            crate::config::validate_id("event", id)?;
            let event = app.get_event(id, None).await?;
            ensure!(event.id == *id, "Event identity differs");
            let hosted = event.default_page_id.is_some()
                || matches!(event.event_type.as_str(), "http" | "simple_chat");
            let mut eligible = event.active
                && event.variant_set().is_empty()
                && event.board_version.is_some_and(concrete)
                && concrete(event.event_version)
                && (hosted || matches!(event.event_type.as_str(), "daemon" | "rest" | "mcp"));
            if eligible {
                eligible = pinned_event(&app, id).await.is_ok();
            }
            let mut readiness_kind = if hosted { "listener" } else { "unsupported" };
            let mut readiness_error = None;
            if eligible && !hosted {
                let version = event.board_version.context("Missing board pin")?;
                let board = Board::from_proto(
                    Board::load_proto(
                        metadata.as_generic(),
                        &flow_like_storage::Path::from("apps").join(project),
                        &event.board_id,
                        Some(version),
                    )
                    .await?,
                );
                match crate::runtime::service_readiness_source(&event, &board) {
                    Ok(Some((
                        _,
                        flow_like_runtime::flow::execution::service::ServiceReadyKind::Daemon,
                    ))) => readiness_kind = "explicit",
                    Ok(Some(_)) => readiness_kind = "listener",
                    Ok(None) => (),
                    Err(error) => {
                        eligible = false;
                        readiness_error = Some(error.to_string());
                    }
                }
            }
            rows.insert(id.clone(), json!({"id": id, "name": event.name.chars().take(120).collect::<String>(), "event_type": event.event_type, "event_version": concrete(event.event_version).then_some(event.event_version), "board_version": event.board_version.filter(|version| concrete(*version)), "hosted": hosted, "eligible": eligible, "readiness_kind": readiness_kind, "rollout_supported": eligible && readiness_kind != "unsupported", "readiness_error": readiness_error}));
        }
        let (items, last) = page(&rows, None)?;
        let more = last
            .as_ref()
            .is_some_and(|last| ids.iter().any(|id| id > last));
        return Ok(
            json!({"project_id":project,"revision":revision,"event_id":null,"items":items,"next":if more { last } else { None }}),
        );
    }
    let (items, last) = page(&rows, after)?;
    let more = last
        .as_ref()
        .is_some_and(|last| rows.keys().any(|id| id > last));
    Ok(
        json!({"project_id":project,"revision":revision,"event_id":event_id,"items":items,"next":if more { last } else { None }}),
    )
}
fn concrete(version: (u32, u32, u32)) -> bool {
    ![version.0, version.1, version.2].contains(&u32::MAX)
}
async fn pinned_event(app: &App, id: &str) -> Result<Event> {
    let live = app.get_event(id, None).await?;
    ensure!(
        live.id == id && concrete(live.event_version),
        "Event has no concrete version"
    );
    let archived = Event::load_pinned(id, app, live.event_version).await?;
    let mut archived_projection = serde_json::to_value(&archived)?;
    let mut live_projection = serde_json::to_value(&live)?;
    for projection in [&mut archived_projection, &mut live_projection] {
        let fields = projection
            .as_object_mut()
            .context("Invalid event metadata")?;
        fields.remove("created_at");
        fields.remove("updated_at");
    }
    ensure!(
        archived_projection == live_projection
            && archived.id == live.id
            && archived.event_version == live.event_version
            && archived.board_id == live.board_id
            && archived.board_version == live.board_version,
        "Pinned event content differs from the current event"
    );
    Ok(archived)
}
fn page(
    rows: &BTreeMap<String, Value>,
    after: Option<&str>,
) -> Result<(Vec<Value>, Option<String>)> {
    let mut items = Vec::new();
    let mut size = 0;
    let mut last = None;
    for (id, row) in rows
        .iter()
        .filter(|(id, _)| after.is_none_or(|cursor| id.as_str() > cursor))
    {
        let bytes = serde_json::to_vec(row)?.len();
        ensure!(bytes <= PAGE_BYTES, "Discovery entry exceeds page limit");
        if size + bytes > PAGE_BYTES {
            break;
        }
        size += bytes;
        items.push(row.clone());
        last = Some(id.clone());
    }
    Ok((items, last))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pages_are_bounded_ordered_and_make_progress() {
        let rows = (0..100)
            .map(|i| {
                (
                    format!("variable-{i:03}"),
                    json!({"id": format!("variable-{i:03}"), "name":"x".repeat(120)}),
                )
            })
            .collect();
        let (first, cursor) = page(&rows, None).unwrap();
        assert!(!first.is_empty() && first.len() < 100);
        let (second, _) = page(&rows, cursor.as_deref()).unwrap();
        assert_eq!(first.len() + second.len(), 100);
        assert!(serde_json::to_vec(&first).unwrap().len() < 12 * 1024);
        assert!(second[0]["id"].as_str().unwrap() > cursor.as_deref().unwrap());
    }
}
