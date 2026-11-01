use super::*;
use crate::functions::a2ui::{page as pages, widget as widgets};
use flow_like::a2ui::widget::{Page, Widget};
use reqwest::Method;
use std::{
    collections::{HashMap, HashSet},
    time::SystemTime,
};

pub(crate) async fn page_list(
    session: &NativePlatformSession,
    app_id: &str,
) -> Result<Vec<Value>, String> {
    if session.is_local(app_id).await? {
        let list = pages::get_pages(session.app_handle.clone(), app_id.to_owned(), None)
            .await
            .map_err(|e| e.to_string())?;
        list.into_iter()
            .map(|entry| serde_json::to_value(entry).map_err(|e| e.to_string()))
            .collect()
    } else {
        session
            .request(
                Method::GET,
                &format!("apps/{}/pages", segment(app_id)),
                None,
            )
            .await?
            .as_array()
            .cloned()
            .ok_or_else(|| "The hub returned an invalid page inventory.".into())
    }
}

pub(crate) async fn load_page(
    session: &NativePlatformSession,
    app_id: &str,
    page_id: &str,
    board_id: Option<&str>,
) -> Result<Page, String> {
    let page: Page = if session.is_local(app_id).await? {
        pages::get_page(
            session.app_handle.clone(),
            app_id.to_owned(),
            page_id.to_owned(),
            board_id.map(str::to_owned),
            None,
        )
        .await
        .map_err(|e| e.to_string())?
    } else {
        let query = board_id
            .map(|id| format!("?board_id={}", segment(id)))
            .unwrap_or_default();
        serde_json::from_value(
            session
                .request(
                    Method::GET,
                    &format!("apps/{}/pages/{}{query}", segment(app_id), segment(page_id)),
                    None,
                )
                .await?,
        )
        .map_err(|e| e.to_string())?
    };
    if page.id != page_id || board_id.is_some_and(|id| page.board_id.as_deref() != Some(id)) {
        return Err("The page identity does not match the requested app and board.".into());
    }
    Ok(page)
}

pub(super) async fn execute(
    session: &NativePlatformSession,
    args: &Value,
    request_id: &str,
) -> Result<Value, String> {
    let captured = widget_surface::snapshot(session);
    let explicit_mode = string(args, "mode", "mode").map(str::to_ascii_lowercase);
    let has_target = [
        "app_id",
        "appId",
        "board_id",
        "boardId",
        "page_id",
        "pageId",
        "page_name",
        "pageName",
        "name",
        "route",
    ]
    .iter()
    .any(|key| string(args, key, key).is_some());
    let mode = explicit_mode
        .as_deref()
        .unwrap_or(if captured.is_some() && !has_target {
            "edit"
        } else {
            "create"
        });
    let mut resolved_args = args.clone();
    if mode == "edit" && string(args, "app_id", "appId").is_none() {
        if let Some(app_id) = captured.as_ref().and_then(|s| s["appId"].as_str()) {
            resolved_args["app_id"] = json!(app_id);
        }
    }
    let app_id = specialist_app_id(session, &resolved_args)?;
    session.require_visible(&app_id)?;
    let requested_board = string(args, "board_id", "boardId");
    if mode == "inspect" {
        let page = load_page(
            session,
            &app_id,
            string(args, "page_id", "pageId").ok_or("inspect requires page_id.")?,
            requested_board,
        )
        .await?;
        let app = board::load_app(session, &app_id)
            .await
            .ok()
            .and_then(|app| serde_json::to_value(app).ok());
        let css = app
            .as_ref()
            .and_then(|app| app.pointer("/frontend/custom_css"))
            .and_then(Value::as_str);
        let page = serde_json::to_value(&page).map_err(|e| e.to_string())?;
        session.assert_current_profile().await?;
        return Ok(super::widget_inspection::inspect(&app_id, &page, css));
    }
    if !["create", "edit"].contains(&mode) {
        return Err("Unknown flowpilot_widget mode.".into());
    }
    let instruction = required(args, "instruction")?.to_owned();
    if mode == "edit" {
        if let Some(surface) = captured
            .as_ref()
            .filter(|surface| matches_surface(surface, &app_id, args))
        {
            return generate_mounted(session, args, surface, request_id).await;
        }
    }
    let inventory = page_list(session, &app_id).await?;
    let explicit_id = string(args, "page_id", "pageId");
    let page_name = string(args, "page_name", "pageName").or_else(|| string(args, "name", "name"));
    let route = string(args, "route", "route");
    let original = if mode == "edit" {
        let mut candidates = Vec::new();
        for entry in &inventory {
            let Some(id) = entry
                .get("pageId")
                .or_else(|| entry.get("id"))
                .and_then(Value::as_str)
            else {
                continue;
            };
            if explicit_id.is_some_and(|target| target != id) {
                continue;
            }
            if requested_board
                .is_some_and(|target| entry.get("boardId").and_then(Value::as_str) != Some(target))
            {
                continue;
            }
            if explicit_id.is_none()
                && page_name
                    .is_some_and(|name| entry.get("name").and_then(Value::as_str) != Some(name))
            {
                continue;
            }
            let page = load_page(session, &app_id, id, requested_board).await?;
            if explicit_id.is_none()
                && route.is_some_and(|route| normalize_route(route) != normalize_route(&page.route))
            {
                continue;
            }
            candidates.push(page);
        }
        if explicit_id.is_none() && page_name.is_none() && route.is_none() {
            return Err(
                "Editing without a mounted builder requires page_id, page_name, or route.".into(),
            );
        }
        if candidates.len() != 1 {
            return Err(format!(
                "The requested page target matched {} pages. Supply an exact page_id.",
                candidates.len()
            ));
        }
        candidates.pop()
    } else {
        None
    };
    if let (Some(original), Some(surface)) = (&original, &captured) {
        if surface["kind"] == "page"
            && surface["appId"] == app_id
            && surface["pageId"] == original.id
        {
            return generate_mounted(session, args, surface, request_id).await;
        }
    }
    let page_id = original
        .as_ref()
        .map(|p| p.id.clone())
        .or_else(|| explicit_id.map(str::to_owned))
        .unwrap_or_else(|| stable_artifact_id(session, request_id, "page", args));
    if mode == "create"
        && inventory
            .iter()
            .any(|entry| entry["pageId"] == page_id || entry["id"] == page_id)
    {
        return Ok(
            json!({"status":"error","code":"FLOWPILOT_WIDGET_PAGE_ALREADY_EXISTS","app_id":app_id,"page_id":page_id,"message":"This exact page already exists. Use mode=edit to change it."}),
        );
    }
    let app = board::load_app(session, &app_id).await?;
    let board_id = original
        .as_ref()
        .and_then(|p| p.board_id.clone())
        .or_else(|| requested_board.map(str::to_owned))
        .or_else(|| (app.boards.len() == 1).then(|| app.boards[0].clone()))
        .or_else(|| {
            app.boards
                .is_empty()
                .then(|| stable_artifact_id(session, request_id, "ui-board", args))
        })
        .ok_or("A page in a multi-board app requires an exact board_id.")?;
    let context = nested_context(
        session,
        request_id,
        Some(app_id.clone()),
        Some(board_id.clone()),
    );
    let components = original.as_ref().map(|page| page.components.clone());
    let canvas = original
        .as_ref()
        .and_then(|page| page.canvas_settings.as_ref())
        .map(serde_json::to_value)
        .transpose()
        .map_err(|e| e.to_string())?;
    let response = run_specialist(
        session,
        CopilotScope::Frontend,
        instruction.clone(),
        false,
        None,
        components,
        canvas,
        context,
        request_id,
        Vec::new(),
    )
    .await?;
    if response.components.is_empty() {
        return Ok(
            json!({"status":"error","code":"FLOWPILOT_WIDGET_NO_COMPONENTS","app_id":app_id,"page_id":page_id,"component_count":0,"applied":false,"specialist_message":response.message,"message":"The UI specialist returned no applicable components. The host did not change the page."}),
        );
    }
    let _app_guard = target_lock(format!("{}:app:{app_id}", session.profile.id)).await;
    let _board_guard =
        target_lock(format!("{}:board:{app_id}:{board_id}", session.profile.id)).await;
    session.assert_current_profile().await?;
    if mode == "edit" {
        if let Some(surface) = widget_surface::snapshot(session).filter(|surface| {
            surface["kind"] == "page" && surface["appId"] == app_id && surface["pageId"] == page_id
        }) {
            return widget_surface::stage(session, &surface, &response, request_id).await;
        }
    }
    let mut page = if let Some(original) = original {
        let fresh = load_page(session, &app_id, &page_id, Some(&board_id)).await?;
        if serde_json::to_value(&original).map_err(|e| e.to_string())?
            != serde_json::to_value(&fresh).map_err(|e| e.to_string())?
        {
            return Ok(
                json!({"status":"error","code":"FLOWPILOT_WIDGET_PAGE_CHANGED","app_id":app_id,"page_id":page_id,"applied":false,"message":"The page changed during generation. Retry the edit against its current contents."}),
            );
        }
        let mut page = fresh;
        page.components = merge_components(&page.components, &response.components)?;
        page
    } else {
        if page_list(session, &app_id)
            .await?
            .iter()
            .any(|entry| entry["pageId"] == page_id || entry["id"] == page_id)
        {
            return Err("The target page was created while the specialist was running. Read it before retrying.".into());
        }
        let current_app = board::load_app(session, &app_id).await?;
        if !current_app.boards.contains(&board_id) {
            board::ensure_board(
                session,
                &app_id,
                &board_id,
                "Main Board",
                &instruction.chars().take(140).collect::<String>(),
            )
            .await?;
        }
        let name = page_name.unwrap_or("New Page");
        let mut page = Page::new(
            &page_id,
            name,
            route
                .map(str::to_owned)
                .unwrap_or_else(|| normalize_route(name)),
        )
        .with_board_id(&board_id);
        page.components = ensure_root(response.components.clone());
        page
    };
    if let Some(canvas) = response.canvas_settings {
        let mut settings = page
            .canvas_settings
            .as_ref()
            .map(serde_json::to_value)
            .transpose()
            .map_err(|e| e.to_string())?
            .unwrap_or_else(|| json!({}));
        let additions = canvas
            .as_object()
            .ok_or("The UI specialist returned invalid canvas settings.")?;
        let settings_object = settings
            .as_object_mut()
            .ok_or("Invalid stored canvas settings.")?;
        settings_object.extend(additions.clone());
        page.canvas_settings = Some(serde_json::from_value(settings).map_err(|e| e.to_string())?);
    }
    page.updated_at = SystemTime::now();
    let created_widgets = persist_inline_widgets(session, &app_id, &mut page, args).await?;
    let target = json!({"app_id":app_id,"board_id":board_id,"page":{"id":page.id,"name":page.name,"route":page.route},"specialist_message":response.message,"specialist_message_timing":"before_host_apply"});
    let saved = persist_page(session, &app_id, &page).await;
    let mut result = target;
    match saved {
        Ok(()) => {
            result["status"] = json!("ok");
            result["applied"] = json!(true);
            result["staged"] = json!(false);
            result["persistence_verified"] = json!(true);
            result["component_count"] = json!(page.components.len());
            result["widgets"] = json!(created_widgets);
            result["message"] = json!(format!(
                "Saved page '{}' and verified its persisted contents.",
                page.name
            ));
        }
        Err(error) => {
            result["status"] = json!("error");
            result["code"] = json!("FLOWPILOT_WIDGET_PERSISTENCE_UNVERIFIED");
            result["outcome"] = json!("unknown");
            result["persistence_verified"] = json!(false);
            result["message"] = json!(format!(
                "The host could not verify the saved page: {error}. Read this exact page before retrying."
            ));
        }
    }
    Ok(result)
}

fn matches_surface(surface: &Value, app_id: &str, args: &Value) -> bool {
    surface["appId"] == app_id
        && string(args, "board_id", "boardId").is_none_or(|id| surface["boardId"] == id)
        && string(args, "page_id", "pageId")
            .is_none_or(|id| surface["kind"] == "page" && surface["pageId"] == id)
        && (string(args, "page_id", "pageId").is_some()
            || string(args, "route", "route").is_none()
                && string(args, "page_name", "pageName")
                    .or_else(|| string(args, "name", "name"))
                    .is_none())
}
async fn generate_mounted(
    session: &NativePlatformSession,
    args: &Value,
    surface: &Value,
    request_id: &str,
) -> Result<Value, String> {
    let app_id = surface["appId"]
        .as_str()
        .ok_or("The builder has no app scope.")?;
    session.require_visible(app_id)?;
    let components =
        serde_json::from_value(surface["currentComponents"].clone()).map_err(|e| e.to_string())?;
    let selection = serde_json::from_value(surface["selectedComponentIds"].clone())
        .map_err(|e| e.to_string())?;
    let context = nested_context(
        session,
        request_id,
        Some(app_id.into()),
        surface["boardId"].as_str().map(str::to_owned),
    );
    let response = run_specialist_with_selection(
        session,
        CopilotScope::Frontend,
        required(args, "instruction")?.into(),
        false,
        None,
        Some(components),
        surface.get("currentCanvasSettings").cloned(),
        Some(selection),
        context,
        request_id,
        Vec::new(),
    )
    .await?;
    if response.components.is_empty() {
        return Ok(
            json!({"status":"error","code":"FLOWPILOT_WIDGET_NO_COMPONENTS","app_id":app_id,"applied":false,"staged":false,"specialist_message":response.message,"message":"The UI specialist returned no applicable components."}),
        );
    }
    widget_surface::stage(session, surface, &response, request_id).await
}

async fn persist_page(
    session: &NativePlatformSession,
    app_id: &str,
    page: &Page,
) -> Result<(), String> {
    session.assert_current_profile().await?;
    if !session.is_local(app_id).await? {
        session
            .request(
                Method::PUT,
                &format!("apps/{}/pages/{}", segment(app_id), segment(&page.id)),
                Some(&json!({"page":page})),
            )
            .await?;
    }
    pages::update_page(session.app_handle.clone(), app_id.to_owned(), page.clone())
        .await
        .map_err(|e| e.to_string())?;
    let readback = load_page(session, app_id, &page.id, page.board_id.as_deref()).await?;
    let expected = serde_json::to_value(page).map_err(|e| e.to_string())?;
    let actual = serde_json::to_value(readback).map_err(|e| e.to_string())?;
    for key in [
        "id",
        "boardId",
        "name",
        "route",
        "components",
        "content",
        "canvasSettings",
        "widgetRefs",
        "layoutType",
    ] {
        if expected.get(key) != actual.get(key) {
            return Err(format!("Authoritative page readback differs at {key}."));
        }
    }
    Ok(())
}

async fn persist_inline_widgets(
    session: &NativePlatformSession,
    app_id: &str,
    page: &mut Page,
    args: &Value,
) -> Result<Vec<Value>, String> {
    let local = session.is_local(app_id).await?;
    let mut requested = Vec::new();
    if let Some(name) = string(args, "widget_name", "widgetName") {
        requested.push(name.to_owned());
    }
    if let Some(names) = args.get("widget_names").and_then(Value::as_array) {
        for name in names.iter().filter_map(Value::as_str) {
            if !name.trim().is_empty() && !requested.iter().any(|held| held == name.trim()) {
                requested.push(name.trim().to_owned());
            }
        }
    }
    let mut existing = if local {
        widgets::get_widgets(session.app_handle.clone(), app_id.to_owned())
            .await
            .map_err(|e| e.to_string())?
    } else {
        let inventory = session
            .request(
                Method::GET,
                &format!("apps/{}/widgets", segment(app_id)),
                None,
            )
            .await?;
        let mut result = Vec::new();
        for item in inventory
            .as_array()
            .ok_or("The widget inventory is invalid.")?
        {
            let id = item
                .get(1)
                .and_then(Value::as_str)
                .or_else(|| item.get("id").and_then(Value::as_str));
            if let Some(id) = id {
                let value = session
                    .request(
                        Method::GET,
                        &format!("apps/{}/widgets/{}", segment(app_id), segment(id)),
                        None,
                    )
                    .await?;
                result.push(serde_json::from_value(value).map_err(|e| e.to_string())?);
            }
        }
        result
    };
    let mut result = Vec::new();
    let mut by_source: HashMap<String, Widget> = HashMap::new();
    for component in &mut page.components {
        if component.get_component_type_name() != "widgetInstance" {
            continue;
        }
        let Some(inline) = component.component.get("inlineWidgetDef").cloned() else {
            continue;
        };
        let source = inline
            .get("id")
            .and_then(Value::as_str)
            .or_else(|| component.component.get("widgetId").and_then(Value::as_str))
            .unwrap_or(&component.id)
            .to_owned();
        let widget = if let Some(widget) = by_source.get(&source) {
            widget.clone()
        } else {
            let name = requested
                .get(result.len())
                .map(String::as_str)
                .or_else(|| inline.get("name").and_then(Value::as_str))
                .unwrap_or("Widget");
            let id = existing
                .iter()
                .find(|widget| name != "Widget" && widget.name.trim() == name)
                .map(|widget| widget.id.clone())
                .unwrap_or_else(|| {
                    format!(
                        "flowpilot-{}",
                        &blake3::hash(format!("{app_id}\0{}\0{source}", page.id).as_bytes())
                            .to_hex()[..24]
                    )
                });
            let mut value =
                serde_json::to_value(Widget::new(&id, name, "root")).map_err(|e| e.to_string())?;
            value.as_object_mut().unwrap().extend(
                inline
                    .as_object()
                    .ok_or("An inline widget definition must be an object.")?
                    .clone(),
            );
            value["id"] = json!(id);
            value["name"] = json!(name);
            value["rootComponentId"] = json!("root");
            let mut widget: Widget = serde_json::from_value(value).map_err(|e| e.to_string())?;
            widget.components = ensure_root(widget.components);
            if !local {
                session
                    .request(
                        Method::PUT,
                        &format!("apps/{}/widgets/{}", segment(app_id), segment(&id)),
                        Some(&json!({"widget":widget})),
                    )
                    .await?;
            }
            widgets::update_widget(
                session.app_handle.clone(),
                app_id.to_owned(),
                widget.clone(),
            )
            .await
            .map_err(|e| e.to_string())?;
            result.push(json!({"id":widget.id,"name":widget.name,"action_ids":widget.actions.iter().map(|action|&action.id).collect::<Vec<_>>() }));
            by_source.insert(source, widget.clone());
            existing.push(widget.clone());
            widget
        };
        component.component["widgetId"] = json!(widget.id);
        page.widget_refs.insert(component.id.clone(), widget);
    }
    Ok(result)
}

fn children(component: &SurfaceComponent) -> Vec<String> {
    component
        .component
        .pointer("/children/explicitList")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}
fn ensure_root(mut components: Vec<SurfaceComponent>) -> Vec<SurfaceComponent> {
    if components.iter().any(|c| c.id == "root") {
        return components;
    }
    let mut referenced = HashSet::new();
    for component in &components {
        referenced.extend(children(component));
        if let Some(id) = component
            .component
            .pointer("/children/template/componentId")
            .and_then(Value::as_str)
        {
            referenced.insert(id.to_owned());
        }
    }
    if let Some(root) = components.iter_mut().find(|c| !referenced.contains(&c.id)) {
        root.id = "root".into();
    }
    components
}
fn merge_components(
    existing: &[SurfaceComponent],
    incoming: &[SurfaceComponent],
) -> Result<Vec<SurfaceComponent>, String> {
    let mut merged = existing.to_vec();
    if !merged.iter().any(|c| c.id == "root") {
        merged.insert(0,serde_json::from_value(json!({"id":"root","style":{"className":"flex-1 h-full overflow-auto"},"component":{"type":"column","gap":"8px","children":{"explicitList":[]}}})).map_err(|e|e.to_string())?);
    }
    let before = merged.iter().find(|c| c.id == "root").unwrap().clone();
    let referenced: HashSet<String> = incoming.iter().flat_map(children).collect();
    let top = incoming
        .iter()
        .filter(|c| c.id != "root" && !referenced.contains(&c.id))
        .map(|c| c.id.clone())
        .collect::<Vec<_>>();
    for component in incoming {
        if let Some(index) = merged.iter().position(|c| c.id == component.id) {
            let mut value = serde_json::to_value(&merged[index]).map_err(|e| e.to_string())?;
            value.as_object_mut().unwrap().extend(
                serde_json::to_value(component)
                    .map_err(|e| e.to_string())?
                    .as_object()
                    .unwrap()
                    .clone(),
            );
            merged[index] = serde_json::from_value(value).map_err(|e| e.to_string())?;
        } else {
            merged.push(component.clone());
        }
    }
    if !top.is_empty() {
        let root = merged.iter_mut().find(|c| c.id == "root").unwrap();
        let mut next = children(&before);
        for id in top {
            if !next.contains(&id) {
                next.push(id);
            }
        }
        root.component = before.component;
        root.component["children"] = json!({"explicitList":next});
    }
    Ok(merged)
}
fn normalize_route(value: &str) -> String {
    let mut slug = String::new();
    for c in value.trim().chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_matches('-');
    format!("/{}", if slug.is_empty() { "page" } else { slug })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn edit_preserves_unmentioned_components_and_links_new_top_level() {
        let root = SurfaceComponent::new(
            "root",
            json!({"type":"column","children":{"explicitList":["old"]}}),
        );
        let old = SurfaceComponent::new("old", json!({"type":"text","content":"kept"}));
        let new = SurfaceComponent::new("new", json!({"type":"text","content":"added"}));
        let merged = merge_components(&[root, old], &[new]).unwrap();
        assert_eq!(merged.len(), 3);
        assert_eq!(children(&merged[0]), vec!["old", "new"]);
        assert_eq!(merged[1].component["content"], "kept");
    }
    #[test]
    fn root_identity_follows_the_unreferenced_component() {
        let child = SurfaceComponent::new("child", json!({"type":"text"}));
        let parent = SurfaceComponent::new(
            "container",
            json!({"type":"column","children":{"explicitList":["child"]}}),
        );
        let result = ensure_root(vec![child, parent]);
        assert_eq!(result[0].id, "child");
        assert_eq!(result[1].id, "root");
    }
    #[test]
    fn route_normalization_matches_page_target_contract() {
        assert_eq!(normalize_route(" /Dashboard View! "), "/dashboard-view");
        assert_eq!(normalize_route("/"), "/page");
    }
}
