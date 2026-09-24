use super::*;
use flow_like_runtime::a2ui::widget::{Page, Widget};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, io::Read};

pub(crate) const MAX_BYTES: u64 = 32 * 1024 * 1024;
const MAX_DOCUMENTS: usize = 1024;

/// Exact bytes are approved by the controller through the authenticated Apply command.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Bundle {
    pub version: u32,
    pub project_id: String,
    pub documents: BTreeMap<String, serde_json::Value>,
}

fn route(kind: &str, id: &str, version: (u32, u32, u32)) -> Result<String> {
    flow_like_device_protocol::validate_instance_identifier(id)?;
    ensure!(
        ![version.0, version.1, version.2].contains(&u32::MAX),
        "Executable metadata must use concrete versions"
    );
    Ok(format!(
        "{kind}/{id}/versions/{}/{}/{}",
        version.0, version.1, version.2
    ))
}

impl Bundle {
    pub(crate) fn load(config: &PlacementConfig) -> Result<Self> {
        let expected = config.online_metadata_sha256.as_deref().context(
            "Online deployment requires controller-approved executable metadata. Prepare and deploy this project again",
        )?;
        flow_like_device_protocol::validate_artifact_digest(expected)?;
        let mut path = config.project_path.clone();
        for part in ["apps", config.project_id.as_str(), "online-metadata.json"] {
            path.push(part);
            ensure!(
                !std::fs::symlink_metadata(&path)?.file_type().is_symlink(),
                "Approved metadata cannot follow symlinks"
            );
        }
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        let file = options.open(&path).context(
            "Approved executable metadata is missing; prepare and deploy this project again",
        )?;
        ensure!(
            file.metadata()?.is_file() && file.metadata()?.len() <= MAX_BYTES,
            "Approved metadata exceeds its size limit"
        );
        let mut bytes = Vec::new();
        file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() as u64 <= MAX_BYTES
                && flow_like_device_protocol::artifact_sha256(&bytes) == expected,
            "Controller-approved executable metadata digest differs"
        );
        let bundle: Self = serde_json::from_slice(&bytes)?;
        ensure!(
            bundle.version == 1
                && bundle.project_id == config.project_id
                && !bundle.documents.is_empty()
                && bundle.documents.len() <= MAX_DOCUMENTS,
            "Invalid approved executable metadata inventory"
        );
        Ok(bundle)
    }

    fn get<T: DeserializeOwned>(&self, path: &str, used: &mut BTreeSet<String>) -> Result<T> {
        let value = self
            .documents
            .get(path)
            .with_context(|| format!("Approved executable metadata is incomplete: {path}"))?;
        used.insert(path.to_owned());
        Ok(serde_json::from_value(value.clone())?)
    }
}

struct BoardPages {
    board: Board,
    pages: Vec<Page>,
    template: bool,
}

/// Validate the entire closure before publishing any object to the runtime store.
fn validate_page_widgets(
    page: &Page,
    widgets: &BTreeMap<String, Widget>,
    project: &str,
) -> Result<()> {
    for content in &page.content {
        let flow_like_runtime::a2ui::widget::PageContent::Widget(instance) = content else {
            continue;
        };
        // An embedded definition belongs to this exact approved Page snapshot. Its
        // historical origin reference never causes a remote lookup at runtime.
        if let Some(embedded) = page.widget_refs.get(&instance.instance_id) {
            ensure!(
                embedded.id == instance.widget_id,
                "Approved embedded widget identity differs"
            );
            continue;
        }
        if let Some(reference) = &instance.widget_ref {
            ensure!(
                reference.app_id == project,
                "Cross-project widget dependencies must be embedded before device deployment"
            );
            ensure!(
                reference.widget_id == instance.widget_id
                    && widgets
                        .get(&reference.widget_id)
                        .is_some_and(|widget| reference
                            .version
                            .is_none_or(|version| widget.version == Some(version))),
                "Approved widget reference is missing or has a different version"
            );
        }
        ensure!(
            widgets.contains_key(&instance.widget_id),
            "Approved Page widget dependency is missing"
        );
    }
    Ok(())
}

struct Approved {
    app: App,
    events: Vec<Event>,
    boards: BTreeMap<String, BoardPages>,
    templates: BTreeMap<String, BoardPages>,
    widgets: BTreeMap<String, Widget>,
}

pub(super) fn validate(config: &PlacementConfig) -> Result<()> {
    prepare(config).map(|_| ())
}

fn prepare(config: &PlacementConfig) -> Result<Approved> {
    let bundle = Bundle::load(config)?;
    let mut used = BTreeSet::new();
    let app: App = bundle.get("app", &mut used)?;
    ensure!(
        app.id == config.project_id
            && !matches!(
                app.visibility,
                flow_like_runtime::app::AppVisibility::Offline
            ),
        "Approved online project identity differs"
    );
    let mut events = Vec::new();
    let mut boards = BTreeMap::new();
    let mut widgets = BTreeMap::new();
    let mut templates = BTreeMap::new();
    for (key, value) in &bundle.documents {
        if key == "app" || key.contains("/pages/") {
            continue;
        }
        let kind = key.split('/').next().unwrap_or_default();
        match kind {
            "events" => {
                let event: Event = serde_json::from_value(value.clone())?;
                ensure!(
                    key == &route("events", &event.id, event.event_version)?,
                    "Approved event identity differs"
                );
                let version = event
                    .board_version
                    .context("Approved event requires a pinned board")?;
                let board_key = route("boards", &event.board_id, version)?;
                ensure!(
                    bundle.documents.contains_key(&board_key),
                    "Approved event board dependency is missing"
                );
                events.push(event);
            }
            "boards" | "templates" => {
                let board: Board = serde_json::from_value(value.clone())?;
                ensure!(
                    key == &route(kind, &board.id, board.version)?,
                    "Approved board identity differs"
                );
                let mut pages = Vec::new();
                let mut page_ids = BTreeSet::new();
                for id in &board.page_ids {
                    flow_like_device_protocol::validate_instance_identifier(id)?;
                    ensure!(page_ids.insert(id), "Duplicate approved Page dependency");
                    let page: Page = bundle.get(&format!("{key}/pages/{id}"), &mut used)?;
                    ensure!(
                        page.id == *id
                            && page
                                .board_id
                                .as_deref()
                                .is_none_or(|owner| kind == "templates" || owner == board.id),
                        "Approved Page identity differs"
                    );
                    pages.push(page);
                }
                let template = kind == "templates";
                let collection = if template {
                    &mut templates
                } else {
                    &mut boards
                };
                // One alias per identity. Different board versions retain their versioned paths.
                ensure!(
                    !template
                        || !collection
                            .values()
                            .any(|entry: &BoardPages| entry.board.id == board.id),
                    "Multiple template versions would make the approved alias ambiguous"
                );
                collection.insert(
                    key.clone(),
                    BoardPages {
                        board,
                        pages,
                        template,
                    },
                );
            }
            "widgets" => {
                let widget: Widget = serde_json::from_value(value.clone())?;
                let version = widget
                    .version
                    .context("Approved widget requires a version")?;
                ensure!(
                    key == &route(kind, &widget.id, version)? && !widgets.contains_key(&widget.id),
                    "Approved widget identity or alias differs"
                );
                widgets.insert(widget.id.clone(), widget);
            }
            _ => anyhow::bail!("Unrecognized approved executable metadata document"),
        }
        used.insert(key.clone());
    }
    ensure!(
        used.len() == bundle.documents.len(),
        "Approved executable metadata contains an unbound Page"
    );
    for id in &app.events {
        ensure!(
            events.iter().any(|event| &event.id == id),
            "Approved event dependency is missing"
        );
    }
    for id in &app.boards {
        ensure!(
            boards.values().any(|entry| &entry.board.id == id),
            "Approved board dependency is missing"
        );
    }
    for id in &app.page_ids {
        ensure!(
            boards
                .values()
                .chain(templates.values())
                .any(|entry| entry.pages.iter().any(|page| &page.id == id)),
            "Approved Page dependency is missing"
        );
    }
    for entry in boards.values().chain(templates.values()) {
        let nodes = entry
            .board
            .nodes
            .values()
            .chain(
                entry
                    .board
                    .layers
                    .values()
                    .flat_map(|layer| layer.nodes.values()),
            )
            .collect::<Vec<_>>();
        for node in &nodes {
            if let Some(references) = &node.fn_refs {
                for reference in &references.fn_refs {
                    ensure!(
                        nodes.iter().any(|target| target.id == *reference),
                        "Approved function dependency is missing"
                    );
                }
            }
        }
        for page in &entry.pages {
            validate_page_widgets(page, &widgets, &app.id)?;
        }
    }
    for id in &app.widget_ids {
        ensure!(
            widgets.contains_key(id),
            "Approved widget dependency is missing"
        );
    }
    for id in &app.templates {
        ensure!(
            templates.values().any(|entry| &entry.board.id == id),
            "Approved template dependency is missing"
        );
    }
    for event in &events {
        let board = boards
            .get(&route(
                "boards",
                &event.board_id,
                event.board_version.context("Missing board version")?,
            )?)
            .context("Missing approved event board")?;
        if let Some(page) = &event.default_page_id {
            ensure!(
                board.board.page_ids.contains(page),
                "Approved Event Page is missing from its board"
            );
        }
    }
    for binding in &config.events {
        ensure!(
            events.iter().any(|event| event.id == binding.event_id
                && [
                    event.event_version.0,
                    event.event_version.1,
                    event.event_version.2
                ] == binding.event_version
                && event.board_version.map(|v| [v.0, v.1, v.2]) == Some(binding.board_version)),
            "Event binding is absent from controller-approved metadata; prepare and deploy this project again"
        );
    }
    for pin in &config.artifact_pins {
        let version = (pin.version[0], pin.version[1], pin.version[2]);
        let matches = match pin.kind {
            crate::config::ArtifactKind::Widget => widgets
                .get(&pin.id)
                .is_some_and(|widget| widget.version == Some(version)),
            crate::config::ArtifactKind::Template => {
                templates.contains_key(&route("templates", &pin.id, version)?)
            }
        };
        ensure!(
            matches,
            "Artifact binding differs from controller-approved metadata"
        );
    }
    Ok(Approved {
        app,
        events,
        boards,
        templates,
        widgets,
    })
}

pub(super) async fn hydrate(config: &PlacementConfig, store: Arc<dyn ObjectStore>) -> Result<()> {
    let Approved {
        app,
        events,
        boards,
        templates,
        widgets,
    } = prepare(config)?;
    let root = ObjectPath::from("apps").child(config.project_id.as_str());
    compress_to_file(store.clone(), root.child("manifest.app"), &app.to_proto()).await?;
    for event in events {
        let path = root
            .child("events")
            .child("versions")
            .child(event.id.as_str())
            .child(format!(
                "{}.{}.{}",
                event.event_version.0, event.event_version.1, event.event_version.2
            ));
        compress_to_file(store.clone(), path, &event.to_proto()).await?;
    }
    for entry in boards.into_values().chain(templates.into_values()) {
        let board = entry.board;
        let (major, minor, patch) = board.version;
        let (path, pages_root) = if entry.template {
            let archive = Board::versioned_template_dir(&root, &board.id);
            compress_to_file(
                store.clone(),
                root.child(format!("{}.template", board.id)),
                &board.to_proto(),
            )
            .await?;
            (
                archive.child(format!("{major}_{minor}_{patch}.template")),
                archive.child(format!("{major}_{minor}_{patch}")),
            )
        } else {
            (
                Board::proto_path(&root, &board.id, Some(board.version)),
                root.child("versions")
                    .child(board.id.as_str())
                    .child(format!("{major}_{minor}_{patch}")),
            )
        };
        compress_to_file(store.clone(), path, &board.to_proto()).await?;
        for page in entry.pages {
            let id = page.id.clone();
            let proto: flow_like_types::proto::Page = page.into();
            compress_to_file(
                store.clone(),
                pages_root.child(format!("{id}.page")),
                &proto,
            )
            .await?;
            if entry.template {
                compress_to_file(
                    store.clone(),
                    Board::template_pages_dir(&root, &board.id).child(format!("{id}.page")),
                    &proto,
                )
                .await?;
            }
        }
    }
    for widget in widgets.into_values() {
        let (major, minor, patch) = widget.version.context("Missing approved widget version")?;
        let path = root
            .child("widgets")
            .child("versions")
            .child(widget.id.as_str())
            .child(format!("{major}-{minor}-{patch}.widget"));
        compress_to_file_json(store.clone(), path, &widget).await?;
        compress_to_file_json(
            store.clone(),
            root.child(format!("{}.widget", widget.id)),
            &widget,
        )
        .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_runtime::a2ui::widget::{PageContent, WidgetInstance, WidgetRef};

    #[test]
    fn approved_embedded_widgets_retain_their_historical_origin() {
        let mut page = Page::new("page", "Page", "/");
        let mut instance = WidgetInstance::new("widget", "instance");
        instance.widget_ref =
            Some(WidgetRef::new("source-project", "widget").with_version(1, 0, 0));
        page.content.push(PageContent::Widget(instance));
        let mut widget = Widget::new("widget", "Copied widget", "root");
        widget.version = Some((1, 0, 0));
        page.widget_refs.insert("instance".into(), widget);
        assert!(validate_page_widgets(&page, &BTreeMap::new(), "project").is_ok());
        page.widget_refs.get_mut("instance").unwrap().id = "another-widget".into();
        assert!(validate_page_widgets(&page, &BTreeMap::new(), "project").is_err());
        page.widget_refs.clear();
        assert!(validate_page_widgets(&page, &BTreeMap::new(), "project").is_err());
    }
}
