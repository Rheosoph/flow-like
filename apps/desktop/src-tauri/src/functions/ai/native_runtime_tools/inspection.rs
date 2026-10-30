use super::super::native_platform_tools::{NativePlatformSession, err, required, segment, string};
use crate::functions::{
    a2ui::{page, widget},
    app, registry,
};
use reqwest::Method;
use serde_json::{Value, json};

fn array(value: Value) -> Result<Vec<Value>, String> {
    value
        .as_array()
        .cloned()
        .ok_or_else(|| "The service returned an invalid inventory.".into())
}
fn encode(value: impl serde::Serialize) -> Result<Value, String> {
    serde_json::to_value(value).map_err(err)
}

pub(super) async fn execute(
    session: &NativePlatformSession,
    args: &Value,
) -> Result<Value, String> {
    let app_id = session.app_id(args)?;
    let board = string(args, "board_id", "boardId").map(str::to_owned);
    let local = session.is_local(&app_id).await?;
    let operation = string(args, "operation", "operation").unwrap_or("list");
    if operation == "page" {
        let id = required(args, "page_id", "pageId")?;
        return Ok(
            json!({"status":"ok","page":summary_page(&read_page(session,&app_id,&id,board.as_deref(),local).await?,None)}),
        );
    }
    let widgets = read_widgets(session, &app_id, local).await?;
    let packages = package_widgets(session, &app_id, local)
        .await
        .unwrap_or_default();
    if operation == "widget" {
        let selector = required(args, "widget_selector", "widgetSelector")?;
        if let Some(package) = packages.iter().find(|p| p["selector"] == selector) {
            return Ok(json!({"status":"ok","package_widget":package}));
        }
        if let Some(widget) = widgets
            .iter()
            .find(|w| w["id"] == selector || w["name"] == selector)
        {
            return Ok(json!({"status":"ok","widget":summary_widget(widget)}));
        }
        if let Some(package) = packages.iter().find(|p| p["name"] == selector) {
            return Ok(json!({"status":"ok","package_widget":package}));
        }
        return Err(format!("Widget '{selector}' not found."));
    }
    if operation == "widgets" {
        return Ok(
            json!({"status":"ok","widgets":widgets.iter().map(summary_widget).collect::<Vec<_>>(),"package_widgets":packages}),
        );
    }
    let list = if local {
        encode(
            page::get_pages(session.app_handle.clone(), app_id.clone(), board.clone())
                .await
                .map_err(err)?,
        )?
    } else {
        session
            .request(
                Method::GET,
                &format!(
                    "apps/{}/pages{}",
                    segment(&app_id),
                    board
                        .as_ref()
                        .map(|id| format!("?board_id={}", segment(id)))
                        .unwrap_or_default()
                ),
                None,
            )
            .await?
    };
    let app_data = if local {
        encode(
            app::get_app(session.app_handle.clone(), app_id.clone())
                .await
                .map_err(err)?,
        )?
    } else {
        session
            .request(Method::GET, &format!("apps/{}", segment(&app_id)), None)
            .await?
    };
    let css = app_data
        .pointer("/frontend/custom_css")
        .and_then(Value::as_str)
        .unwrap_or("");
    let mut pages = Vec::new();
    for entry in array(list)? {
        let id = string(&entry, "page_id", "pageId").unwrap_or("");
        let entry_board = string(&entry, "board_id", "boardId").or(board.as_deref());
        let page = match read_page(session, &app_id, id, entry_board, local).await {
            Ok(page) => summary_page(&page, Some(css)),
            Err(_) => {
                json!({"page_id":id,"name":entry.get("name"),"element_refs":[],"error":"Page details could not be loaded."})
            }
        };
        pages.push(page);
    }
    Ok(
        json!({"status":"ok","app_id":app_id,"board_id":board,"complete":pages.iter().all(|p|p.get("error").is_none()),"pages":pages,"widgets":widgets.iter().map(|w|json!({"selector":w.get("name"),"widget_id":w.get("id"),"description":w.get("description")})).collect::<Vec<_>>(),"package_widgets":packages,"note":"This listing does not include widget binding pins. Call ui_inspect with operation 'widget' for a widget's instantiate_pins before authoring any dyn* argument."}),
    )
}

async fn read_page(
    session: &NativePlatformSession,
    app: &str,
    id: &str,
    board: Option<&str>,
    local: bool,
) -> Result<Value, String> {
    if local {
        encode(
            page::get_page(
                session.app_handle.clone(),
                app.into(),
                id.into(),
                board.map(str::to_owned),
                None,
            )
            .await
            .map_err(err)?,
        )
    } else {
        session
            .request(
                Method::GET,
                &format!(
                    "apps/{}/pages/{}{}",
                    segment(app),
                    segment(id),
                    board
                        .map(|id| format!("?board_id={}", segment(id)))
                        .unwrap_or_default()
                ),
                None,
            )
            .await
    }
}
async fn read_widgets(
    session: &NativePlatformSession,
    app: &str,
    local: bool,
) -> Result<Vec<Value>, String> {
    if local {
        return array(encode(
            widget::get_widgets(session.app_handle.clone(), app.into())
                .await
                .map_err(err)?,
        )?);
    }
    let entries = array(
        session
            .request(Method::GET, &format!("apps/{}/widgets", segment(app)), None)
            .await?,
    )?;
    let mut widgets = Vec::new();
    for entry in entries {
        let id = entry
            .get(1)
            .and_then(Value::as_str)
            .or_else(|| entry.get("id").and_then(Value::as_str))
            .ok_or("Invalid widget list entry")?;
        let widget = session
            .request(
                Method::GET,
                &format!("apps/{}/widgets/{}", segment(app), segment(id)),
                None,
            )
            .await?;
        widgets.push(widget);
    }
    Ok(widgets)
}

fn summary_page(page: &Value, app_css: Option<&str>) -> Value {
    let id = page.get("id").and_then(Value::as_str).unwrap_or("");
    json!({"page_id":id,"name":page.get("name"),"route":page.get("route"),"on_load_event_id":page.get("onLoadEventId"),"on_interval_event_id":page.get("onIntervalEventId"),"custom_css_chars":page.pointer("/canvasSettings/customCss").and_then(Value::as_str).unwrap_or("").encode_utf16().count(),"app_custom_css_chars":app_css.map(|css|css.encode_utf16().count()),"element_refs":page.get("components").and_then(Value::as_array).into_iter().flatten().filter_map(|c|c.get("id").and_then(Value::as_str)).map(|component|format!("{id}/{component}")).collect::<Vec<_>>()})
}

fn pin(kind: &str, key: &str) -> String {
    let input = format!("dyn_{kind}_{key}");
    let mut result = String::new();
    let mut upper = false;
    for c in input.chars() {
        if c.is_alphanumeric() {
            if result.is_empty() {
                result.extend(c.to_lowercase());
            } else if upper {
                result.extend(c.to_uppercase());
            } else {
                result.push(c);
            }
            upper = false;
        } else if !result.is_empty() {
            upper = true;
        }
    }
    result
}
fn bound_paths(value: &Value, paths: &mut Vec<String>) {
    match value {
        Value::Array(values) => {
            for value in values {
                bound_paths(value, paths)
            }
        }
        Value::Object(object) => {
            if let Some(path) = object.get("path").and_then(Value::as_str) {
                if !path.is_empty() && !paths.iter().any(|p| p == path) {
                    paths.push(path.into());
                }
                return;
            }
            for value in object.values() {
                bound_paths(value, paths);
            }
        }
        _ => {}
    }
}
fn summary_widget(widget: &Value) -> Value {
    let mut paths = Vec::new();
    for component in widget
        .get("components")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(value) = component.get("component") {
            bound_paths(value, &mut paths);
        }
    }
    let mut pins = paths
        .iter()
        .map(|path| json!({"pin":pin("path",path),"bound_path":path}))
        .collect::<Vec<_>>();
    for (key, kind) in [("exposedProps", "prop"), ("customizationOptions", "cust")] {
        for value in widget
            .get(key)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(id) = value.get("id").and_then(Value::as_str) else {
                continue;
            };
            pins.push(if kind=="prop" {json!({"pin":pin(kind,id),"label":value.get("label"),"property_path":value.get("propertyPath")})} else {json!({"pin":pin(kind,id),"label":value.get("label"),"customization":id})});
        }
    }
    json!({"selector":widget.get("name"),"widget_id":widget.get("id"),"description":widget.get("description"),"instantiate_pins":pins,"actions":widget.get("actions").and_then(Value::as_array).into_iter().flatten().filter_map(|action|action.get("id").and_then(Value::as_str)).filter(|id|!id.is_empty()).collect::<Vec<_>>()})
}
async fn package_widgets(
    session: &NativePlatformSession,
    app_id: &str,
    local: bool,
) -> Result<Vec<Value>, String> {
    let packages = if local {
        encode(
            app::app_list_packages(session.app_handle.clone(), app_id.into())
                .await
                .map_err(err)?,
        )?
    } else {
        session
            .request(
                Method::GET,
                &format!("apps/{}/packages", segment(app_id)),
                None,
            )
            .await?
    };
    let mut result = Vec::new();
    for (id, version) in packages.as_object().into_iter().flatten() {
        let Some(installed) =
            registry::registry_get_package(session.app_handle.clone(), id.clone())
                .await
                .map_err(err)?
        else {
            continue;
        };
        let installed = encode(installed)?;
        let manifest = installed.get("manifest").unwrap_or(&installed);
        for widget in manifest
            .get("widgets")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(widget_id) = widget.get("id").and_then(Value::as_str) else {
                continue;
            };
            let contract = widget.get("contract").cloned().unwrap_or(json!({}));
            let pins=contract.get("inputs").and_then(Value::as_object).into_iter().flatten().map(|(key,value)|json!({"pin":pin("in",key),"input":key,"optional":value.get("optional")})).collect::<Vec<_>>();
            result.push(json!({"selector":format!("pkg:{id}/{widget_id}"),"package_id":id,"widget_id":widget_id,"package_version":version,"bundle_hash":manifest.get("widget_bundle_hash").or(manifest.get("widgetBundleHash")),"name":widget.get("name"),"description":widget.get("description"),"contract":contract,"instantiate_pins":pins}));
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn widget_inspection_exposes_exact_binding_and_action_ids() {
        let summary = summary_widget(
            &json!({"id":"w","name":"Cards","components":[{"component":{"text":{"path":"/person/name"}}}],"exposedProps":[{"id":"primary-color","propertyPath":"style.color"}],"actions":[{"id":"submit","name":"wrong"}]}),
        );
        assert_eq!(summary["instantiate_pins"][0]["pin"], "dynPathPersonName");
        assert_eq!(summary["instantiate_pins"][1]["pin"], "dynPropPrimaryColor");
        assert_eq!(summary["actions"], json!(["submit"]));
    }
}
