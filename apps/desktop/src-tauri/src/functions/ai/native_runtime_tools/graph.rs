use super::super::native_platform_tools::{
    NativePlatformSession, boolean, err, required, segment, string,
};
use crate::functions::app::graph as service;
use reqwest::Method;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

fn decode<T: DeserializeOwned>(value: Value) -> Result<T, String> {
    serde_json::from_value(value).map_err(err)
}
fn encode(value: impl Serialize) -> Result<Value, String> {
    serde_json::to_value(value).map_err(err)
}

/// These tool operations cannot change governed action definitions or public exposure.
fn overlay_payload(args: &Value, create: bool) -> Value {
    let mut payload = serde_json::Map::new();
    for key in [
        "name",
        "description",
        "nodes",
        "edges",
        "object_views",
        "bindings_enabled",
        "default_limit",
        "expected_updated_at",
    ] {
        if let Some(value) = args.get(key) {
            payload.insert(key.into(), value.clone());
        }
    }
    if create {
        for key in ["nodes", "edges", "object_views"] {
            payload.entry(key).or_insert_with(|| json!([]));
        }
        payload.entry("name").or_insert_with(|| json!(""));
        payload
            .entry("bindings_enabled")
            .or_insert_with(|| json!(false));
    }
    Value::Object(payload)
}

pub(super) async fn execute(
    session: &NativePlatformSession,
    tool: &str,
    args: &Value,
) -> Result<Value, String> {
    let app_id = session.app_id(args)?;
    let operation = required(args, "operation", "operation")?;
    let overlay = string(args, "overlay_id", "overlayId")
        .unwrap_or("")
        .to_owned();
    let user = Some(boolean(args, "user_scoped", "userScoped", false));
    let local = session.is_local(&app_id).await?;
    let app = session.app_handle.clone();
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .map(|value| value.min(10_000) as usize);
    let base = format!("apps/{}/graph", segment(&app_id));
    let target = format!("{base}/{}", segment(&overlay));
    let scope = if user == Some(true) {
        "?scope=user"
    } else {
        ""
    };
    session.assert_current_profile().await?;
    if tool == "ontology_action_tool" {
        return ontology(session, args, &app_id, &overlay, local, user).await;
    }
    let (field, result) = match (tool, operation.as_str()) {
        ("graph_overlay_tool", "list_overlays") => (
            "overlays",
            if local {
                encode(
                    service::graph_list_overlays(app, app_id, user)
                        .await
                        .map_err(err)?,
                )?
            } else {
                session
                    .request(Method::GET, &format!("{base}{scope}"), None)
                    .await?
            },
        ),
        ("graph_overlay_tool", "get_overlay") => (
            "overlay",
            if local {
                encode(
                    service::graph_get_overlay(app, app_id, overlay, user)
                        .await
                        .map_err(err)?,
                )?
            } else {
                session
                    .request(Method::GET, &format!("{target}{scope}"), None)
                    .await?
            },
        ),
        ("graph_overlay_tool", "get_schema") => (
            "schema",
            if local {
                service::graph_get_schema(app, app_id, overlay, user)
                    .await
                    .map_err(err)?
            } else {
                session
                    .request(Method::GET, &format!("{target}/schema{scope}"), None)
                    .await?
            },
        ),
        ("graph_overlay_tool", "validate_overlay") => (
            "validation",
            if local {
                service::graph_validate_overlay(
                    app,
                    app_id,
                    overlay,
                    user,
                    args.get("draft").cloned().map(decode).transpose()?,
                )
                .await
                .map_err(err)?
            } else {
                session
                    .request(
                        Method::POST,
                        &format!("{target}/validate{scope}"),
                        args.get("draft"),
                    )
                    .await?
            },
        ),
        ("graph_overlay_tool", "create_overlay") => {
            let payload = overlay_payload(args, true);
            (
                "overlay",
                if local {
                    encode(
                        service::graph_create_overlay(app, app_id, decode(payload)?, user)
                            .await
                            .map_err(err)?,
                    )?
                } else {
                    session
                        .request(Method::POST, &format!("{base}{scope}"), Some(&payload))
                        .await?
                },
            )
        }
        ("graph_overlay_tool", "update_overlay") => {
            let payload = overlay_payload(args, false);
            (
                "overlay",
                if local {
                    encode(
                        service::graph_update_overlay(app, app_id, overlay, decode(payload)?, user)
                            .await
                            .map_err(err)?,
                    )?
                } else {
                    session
                        .request(Method::PUT, &format!("{target}{scope}"), Some(&payload))
                        .await?
                },
            )
        }
        ("graph_overlay_tool", "delete_overlay") => {
            if local {
                service::graph_delete_overlay(app, app_id, overlay, user)
                    .await
                    .map_err(err)?;
            } else {
                session
                    .request(Method::DELETE, &format!("{target}{scope}"), None)
                    .await?;
            }
            return Ok(json!({"status":"ok"}));
        }
        ("graph_query_tool", "cypher" | "sql") => {
            let payload = json!({"query":string(args,"query","query").unwrap_or(""),"params":args.get("params"),"limit":limit});
            (
                "rows",
                if local {
                    if operation == "cypher" {
                        service::graph_cypher(app, app_id, overlay, decode(payload)?, user)
                            .await
                            .map_err(err)?
                    } else {
                        service::graph_sql(app, app_id, overlay, decode(payload)?, user)
                            .await
                            .map_err(err)?
                    }
                } else {
                    session
                        .request(
                            Method::POST,
                            &format!("{target}/{operation}{scope}"),
                            Some(&payload),
                        )
                        .await?
                },
            )
        }
        ("graph_query_tool", "neighbors" | "subgraph" | "paths" | "search_nodes") => {
            let mut payload = args.clone();
            if let Some(object) = payload.as_object_mut() {
                object.insert("limit".into(), json!(limit));
            }
            let result = if local {
                match operation.as_str() {
                    "neighbors" => {
                        service::graph_neighbors(app, app_id, overlay, decode(payload)?, user).await
                    }
                    "subgraph" => {
                        service::graph_subgraph(app, app_id, overlay, decode(payload)?, user).await
                    }
                    "paths" => {
                        service::graph_paths(app, app_id, overlay, decode(payload)?, user).await
                    }
                    _ => {
                        service::graph_search_nodes(app, app_id, overlay, decode(payload)?, user)
                            .await
                    }
                }
                .map_err(err)?
            } else {
                let route = if operation == "search_nodes" {
                    "search"
                } else {
                    &operation
                };
                session
                    .request(
                        Method::POST,
                        &format!("{target}/{route}{scope}"),
                        Some(&payload),
                    )
                    .await?
            };
            (
                if operation == "search_nodes" {
                    "nodes"
                } else {
                    "result"
                },
                result,
            )
        }
        ("graph_query_tool", "analytics" | "sample") => {
            let label = string(args, "label", "label").unwrap_or("").to_owned();
            let n = args
                .get("n")
                .and_then(Value::as_u64)
                .map(|n| n.min(500) as usize);
            let result = if local {
                if operation == "analytics" {
                    service::graph_analytics(app, app_id, overlay, limit, user).await
                } else {
                    service::graph_sample(app, app_id, overlay, label, n, user).await
                }
                .map_err(err)?
            } else {
                let mut query_url = reqwest::Url::parse("https://query.invalid/").map_err(err)?;
                {
                    let mut query = query_url.query_pairs_mut();
                    if user == Some(true) {
                        query.append_pair("scope", "user");
                    }
                    if let Some(limit) = limit {
                        query.append_pair("limit", &limit.to_string());
                    }
                    if operation == "sample" {
                        query.append_pair("label", &label);
                        if let Some(n) = n {
                            query.append_pair("n", &n.to_string());
                        }
                    }
                }
                session
                    .request(
                        Method::GET,
                        &format!("{target}/{operation}?{}", query_url.query().unwrap_or("")),
                        None,
                    )
                    .await?
            };
            (
                if operation == "analytics" {
                    "analytics"
                } else {
                    "rows"
                },
                result,
            )
        }
        ("graph_element_tool", "add_nodes" | "add_edges") => {
            let payload = json!({"label":required(args,"label","label")?, "rows":args.get("rows").cloned().unwrap_or(json!([]))});
            let mut result = if local {
                if operation == "add_nodes" {
                    service::graph_upsert_nodes(app, app_id, overlay, decode(payload)?, user).await
                } else {
                    service::graph_upsert_edges(app, app_id, overlay, decode(payload)?, user).await
                }
                .map_err(err)?
            } else {
                let route = if operation == "add_nodes" {
                    "nodes"
                } else {
                    "edges"
                };
                session
                    .request(
                        Method::POST,
                        &format!("{target}/{route}{scope}"),
                        Some(&payload),
                    )
                    .await?
            };
            result
                .as_object_mut()
                .ok_or("Graph write returned an invalid result")?
                .insert("status".into(), json!("ok"));
            return Ok(result);
        }
        _ => return Err(format!("Unsupported {tool} operation '{operation}'.")),
    };
    Ok(json!({"status":"ok",field:result}))
}

async fn ontology(
    session: &NativePlatformSession,
    args: &Value,
    app_id: &str,
    overlay: &str,
    local: bool,
    user: Option<bool>,
) -> Result<Value, String> {
    let operation = required(args, "operation", "operation")?;
    let action_id = string(args, "action_id", "actionId").unwrap_or("");
    let base = format!("apps/{}/graph/{}", segment(app_id), segment(overlay));
    if operation == "prerun_action" {
        let prerun = if local {
            json!({"oauth_requirements":[],"signature":""})
        } else {
            session
                .request(
                    Method::GET,
                    &format!("{base}/actions/{}/prerun", segment(action_id)),
                    None,
                )
                .await?
        };
        return Ok(json!({"status":"ok","prerun":prerun}));
    }
    if operation == "invoke_action" {
        if local {
            let prepared = service::graph_prepare_ontology_action(
                session.app_handle.clone(),
                app_id.into(),
                overlay.into(),
                action_id.into(),
                decode(args.clone())?,
            )
            .await
            .map_err(err)?;
            let result = super::execution::execute(
                session,
                "execute_event",
                &json!({"app_id":app_id,"event_id":prepared.event_id,"payload":prepared.payload}),
            )
            .await?;
            return Ok(json!({"status":"ok","run":{
                "run_id":result.get("run_id"),
                "status":if result.pointer("/metadata/log_level").and_then(Value::as_u64).unwrap_or(0)>=3 {"Failed"} else {"Completed"}
            }}));
        }
        return super::execution::invoke_ontology(session, args, &base, action_id).await;
    }
    let data = if local {
        encode(
            service::graph_get_overlay(
                session.app_handle.clone(),
                app_id.into(),
                overlay.into(),
                user,
            )
            .await
            .map_err(err)?,
        )?
    } else {
        session.request(Method::GET, &base, None).await?
    };
    let actions = data
        .get("actions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    match operation.as_str() {
        "list_actions" => Ok(
            json!({"status":"ok","actions": actions.into_iter().map(|action| {
            let mut result = serde_json::Map::new();
            for key in ["id","name","description","object_type","enabled","allow_bulk"] {
                if let Some(value)=action.get(key) { result.insert(key.into(),value.clone()); }
            }
            Value::Object(result)
        }).collect::<Vec<_>>()}),
        ),
        "describe_action" => {
            let action = actions
                .into_iter()
                .find(|value| value.get("id").and_then(Value::as_str) == Some(action_id));
            let prerun = if local {
                json!({"oauth_requirements":[],"signature":""})
            } else {
                session
                    .request(
                        Method::GET,
                        &format!("{base}/actions/{}/prerun", segment(action_id)),
                        None,
                    )
                    .await?
            };
            Ok(json!({"status":"ok","action":action,"prerun":prerun}))
        }
        _ => Err(format!(
            "Unsupported ontology action operation '{operation}'."
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tool_overlay_writes_cannot_change_governed_fields() {
        let args = json!({"name":"x","actions":[{"id":"run"}],"exposed":true,"nodes":[]});
        for create in [true, false] {
            let payload = overlay_payload(&args, create);
            assert!(payload.get("actions").is_none());
            assert!(payload.get("exposed").is_none());
            assert_eq!(payload["name"], "x");
        }
    }
}
