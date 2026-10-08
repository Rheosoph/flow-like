use std::{collections::HashSet, sync::Arc};

use flow_like::flow::{
    execution::{context::ExecutionContext, internal_pin::InternalPin},
    node::{Node, NodeLogic},
    pin::{PinType, ValueType, resolve_schema},
    variable::{VariableType, effective_default},
};
use flow_like_catalog_core::{FlowPath, UploadedFile, materialize_uploaded_files};
use flow_like_types::{Value, async_trait, json::json};
pub mod push_generic_result;

fn normalize_key(key: &str) -> String {
    key.to_lowercase().replace('_', "")
}

fn find_matching_key(
    obj: &flow_like_types::json::Map<String, flow_like_types::Value>,
    pin_name: &str,
) -> Option<String> {
    let normalized_pin = normalize_key(pin_name);

    obj.keys()
        .find(|key| normalize_key(key) == normalized_pin)
        .cloned()
}

fn collect_pins(context: &ExecutionContext) -> (Vec<Arc<InternalPin>>, Vec<Arc<InternalPin>>) {
    let mut exec_pins = Vec::new();
    let mut output_pins = Vec::new();

    for pin in context.node.pins.iter() {
        if pin.pin_type == PinType::Output {
            if pin.data_type == VariableType::Execution {
                exec_pins.push(pin.clone());
            } else if pin.name.as_ref() != "payload" {
                output_pins.push(pin.clone());
            }
        }
    }

    (exec_pins, output_pins)
}

fn missing_pin_value(optional: bool, pin: &InternalPin) -> Option<Value> {
    if !optional {
        return None;
    }
    Some(effective_default(
        pin.default_value.as_deref(),
        &pin.data_type,
        &pin.value_type,
        pin.schema.as_deref(),
    ))
}

async fn optional_pin_ids(context: &ExecutionContext) -> HashSet<String> {
    context
        .read_node()
        .await
        .pins
        .into_iter()
        .filter(|(_, pin)| {
            pin.pin_type == PinType::Output
                && pin.options.as_ref().and_then(|options| options.optional) == Some(true)
        })
        .map(|(id, _)| id)
        .collect()
}

fn is_flow_path_schema(schema: &Value) -> bool {
    if schema.get("title").and_then(Value::as_str) == Some("FlowPath") {
        return true;
    }
    if schema.get("type").and_then(Value::as_str) == Some("array") {
        return schema.get("items").is_some_and(is_flow_path_schema);
    }
    let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
        return false;
    };
    let Some(required) = schema.get("required").and_then(Value::as_array) else {
        return false;
    };
    ["path", "store_ref"].iter().all(|name| {
        properties.contains_key(*name) && required.iter().any(|value| value.as_str() == Some(*name))
    })
}

fn uploaded_file(value: &Value) -> Option<UploadedFile> {
    let url = value.get("url")?.as_str().filter(|url| !url.is_empty())?;
    let flow_path = flow_like_types::json::from_value::<FlowPath>(value.clone()).ok()?;
    Some(UploadedFile {
        flow_path: Some(flow_path),
        url: Some(url.to_string()),
        name: value
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_string),
        ..Default::default()
    })
}

/// Form uploads retain their source URL until the Event can hand them to this run's store.
async fn materialize_pin_uploads(
    context: &mut ExecutionContext,
    pin: &InternalPin,
    value: &Value,
) -> flow_like_types::Result<Value> {
    if pin.data_type != VariableType::Struct {
        return Ok(value.clone());
    }
    let collection = matches!(pin.value_type, ValueType::Array | ValueType::HashSet);
    let values = if collection {
        match value.as_array() {
            Some(values) => values.as_slice(),
            None => return Ok(value.clone()),
        }
    } else if pin.value_type == ValueType::Normal {
        std::slice::from_ref(value)
    } else {
        return Ok(value.clone());
    };
    let (indices, mut files): (Vec<_>, Vec<_>) = values
        .iter()
        .enumerate()
        .filter_map(|(index, value)| uploaded_file(value).map(|file| (index, file)))
        .unzip();
    if files.is_empty() {
        return Ok(value.clone());
    }
    let Some(schema) = pin.schema.as_deref() else {
        return Ok(value.clone());
    };
    let schema = {
        let board = context.get_board().await?;
        let schema = resolve_schema(schema, &board.refs)?;
        flow_like_types::json::from_str::<Value>(schema).ok()
    };
    if !schema.as_ref().is_some_and(is_flow_path_schema) {
        return Ok(value.clone());
    }

    // Form sources are signed upload URLs. Local picks already carry a staged FlowPath.
    for file in &files {
        let url = file.url.as_deref().unwrap_or_default();
        let remote_url = flow_like_types::reqwest::Url::parse(url)
            .ok()
            .is_some_and(|url| {
                matches!(url.scheme(), "http" | "https")
                    && url.host_str() != Some("asset.localhost")
            });
        if !remote_url {
            return Err(flow_like_types::anyhow!(
                "Form file input '{}' requires an HTTP or HTTPS upload URL",
                pin.name()
            ));
        }
    }

    let paths = materialize_uploaded_files(context, pin.id(), &mut files).await?;
    let mut prepared = values.to_vec();
    for (index, path) in indices.into_iter().zip(paths) {
        prepared[index] = json!(path);
    }
    Ok(if collection {
        Value::Array(prepared)
    } else {
        prepared.remove(0)
    })
}

async fn try_match_and_set_pin(
    context: &mut ExecutionContext,
    obj: &flow_like_types::json::Map<String, flow_like_types::Value>,
    pin: &Arc<InternalPin>,
) -> flow_like_types::Result<Option<String>> {
    let pin_name = pin.name();
    if let Some(value) = obj.get(pin_name) {
        let value = materialize_pin_uploads(context, pin, value).await?;
        context.set_pin_ref_value(pin, value).await?;
        return Ok(Some(pin_name.to_string()));
    }

    if let Some(key) = find_matching_key(obj, pin_name)
        && let Some(value) = obj.get(&key)
    {
        let value = materialize_pin_uploads(context, pin, value).await?;
        context.set_pin_ref_value(pin, value).await?;
        return Ok(Some(key));
    }

    Ok(None)
}

async fn map_payload_to_pins(
    context: &mut ExecutionContext,
    obj: &mut flow_like_types::json::Map<String, flow_like_types::Value>,
    output_pins: &[Arc<InternalPin>],
) -> flow_like_types::Result<Vec<Arc<InternalPin>>> {
    let mut matched_keys = Vec::new();
    let mut unmatched = Vec::new();

    for pin in output_pins {
        match try_match_and_set_pin(context, obj, pin).await? {
            Some(key) => matched_keys.push(key),
            None => unmatched.push(pin.clone()),
        }
    }

    for key in matched_keys {
        obj.remove(&key);
    }

    Ok(unmatched)
}

async fn fill_missing_pins(context: &mut ExecutionContext, unmatched: &[Arc<InternalPin>]) {
    if unmatched.is_empty() {
        return;
    }

    let optional = optional_pin_ids(context).await;
    for pin in unmatched {
        if let Some(value) = missing_pin_value(optional.contains(pin.id()), pin) {
            // Written on the pin directly: Generic and Point/LineString/Polygon pins have no
            // empty shape and resolve to Null, which typed validation would refuse as unset.
            context.override_pin_value_if_active(pin.id(), &value);
            pin.set_value(value).await;
        }
    }
}

/// Delegated callers (agent tools, in-process MCP/REST servers, Call Function) write the
/// arguments they received straight onto the output pins; whatever is still unset was
/// not provided.
async fn unset_pins(output_pins: &[Arc<InternalPin>]) -> Vec<Arc<InternalPin>> {
    let mut unset = Vec::new();
    for pin in output_pins {
        if pin.get_raw_value().await.is_none() {
            unset.push(pin.clone());
        }
    }
    unset
}

async fn activate_all_exec_pins(
    context: &ExecutionContext,
    exec_pins: Vec<Arc<InternalPin>>,
) -> flow_like_types::Result<()> {
    for exec_pin in exec_pins {
        context.activate_exec_pin_ref(&exec_pin).await?;
    }
    Ok(())
}

async fn process_payload(
    context: &mut ExecutionContext,
    output_pins: &[Arc<InternalPin>],
) -> flow_like_types::Result<()> {
    let payload_data = context.get_payload().await?;
    let mut payload = payload_data
        .payload
        .clone()
        .unwrap_or_else(|| flow_like_types::Value::Object(flow_like_types::json::Map::new()));

    let unmatched = if let Some(obj) = payload.as_object_mut() {
        let unmatched = map_payload_to_pins(context, obj, output_pins).await?;
        context.set_pin_value("payload", json!(obj)).await?;
        unmatched
    } else {
        context.set_pin_value("payload", payload).await?;
        output_pins.to_vec()
    };

    fill_missing_pins(context, &unmatched).await;

    Ok(())
}

#[crate::register_node]
#[derive(Default)]
pub struct GenericEventNode {}

impl GenericEventNode {
    pub fn new() -> Self {
        GenericEventNode {}
    }
}

#[async_trait]
impl NodeLogic for GenericEventNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "events_generic",
            "Generic Event",
            "A generic event without input or output",
            "Events",
        );
        node.set_flowscript_name("events", "generic");
        node.add_icon("/flow/icons/event.svg");
        node.set_start(true);
        node.set_can_be_referenced_by_fns(true);

        node.add_output_pin(
            "exec_out",
            "Exec Out",
            "Starting an event",
            VariableType::Execution,
        );

        node.add_output_pin(
            "payload",
            "Payload",
            "The payload of the event",
            VariableType::Struct,
        )
        .set_open_schema();

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let (exec_pins, output_pins) = collect_pins(context);

        if context.delegated {
            let unset = unset_pins(&output_pins).await;
            fill_missing_pins(context, &unset).await;
            return activate_all_exec_pins(context, exec_pins).await;
        }

        process_payload(context, &output_pins).await?;
        activate_all_exec_pins(context, exec_pins).await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like::{
        flow::{
            board::{Board, ExecutionStage},
            execution::{InternalRun, LogLevel, RunPayload, RunStatus},
            pin::{PinOptions, ValueType},
        },
        profile::Profile,
        state::{FlowLikeConfig, FlowLikeState, FlowNodeRegistryInner},
        utils::http::HTTPClient,
    };
    use flow_like_storage::{Path, files::store::FlowLikeStore};
    use flow_like_types::{
        geometry::{GeometryKind, marker},
        intercom::BufferedInterComHandler,
        sync::RwLock,
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const EVENT_ID: &str = "event";

    fn internal_pin(data_type: VariableType, default: Option<Value>) -> InternalPin {
        let mut node = Node::new("pin-test", "Pin", "", "");
        let pin = node
            .add_output_pin("pin", "Pin", "", data_type)
            .set_default_value(default)
            .clone();
        InternalPin::new(&pin, false)
    }

    #[test]
    fn required_pins_are_never_filled() {
        assert_eq!(
            missing_pin_value(false, &internal_pin(VariableType::String, None)),
            None
        );
        assert_eq!(
            missing_pin_value(
                false,
                &internal_pin(VariableType::String, Some(json!("fallback")))
            ),
            None
        );
    }

    #[test]
    fn optional_pins_fall_back_to_the_type_default_without_a_usable_default() {
        assert_eq!(
            missing_pin_value(true, &internal_pin(VariableType::Integer, None)),
            Some(json!(0))
        );
        assert_eq!(
            missing_pin_value(
                true,
                &internal_pin(VariableType::Integer, Some(Value::Null))
            ),
            Some(json!(0))
        );
        assert_eq!(
            missing_pin_value(true, &internal_pin(VariableType::String, None)),
            Some(json!(""))
        );
        assert_eq!(
            missing_pin_value(true, &internal_pin(VariableType::Struct, None)),
            Some(json!({}))
        );
        assert_eq!(
            missing_pin_value(true, &internal_pin(VariableType::Generic, None)),
            Some(Value::Null)
        );
    }

    #[test]
    fn optional_pins_take_their_default() {
        assert_eq!(
            missing_pin_value(
                true,
                &internal_pin(VariableType::Struct, Some(json!({"nested": [1, 2]})))
            ),
            Some(json!({"nested": [1, 2]}))
        );
        assert_eq!(
            missing_pin_value(true, &internal_pin(VariableType::Integer, Some(json!(0)))),
            Some(json!(0))
        );
        assert_eq!(
            missing_pin_value(
                true,
                &internal_pin(VariableType::Boolean, Some(json!(false)))
            ),
            Some(json!(false))
        );
    }

    fn event_node() -> Node {
        let mut node = GenericEventNode::new().get_node();
        node.id = EVENT_ID.to_string();
        let optional = PinOptions::new().set_optional(true).build();
        node.add_output_pin("name", "Name", "", VariableType::String)
            .set_default_value(Some(json!("anonymous")))
            .set_options(optional.clone());
        node.add_output_pin("count", "Count", "", VariableType::Integer)
            .set_options(optional.clone());
        node.add_output_pin("required", "Required", "", VariableType::String)
            .set_default_value(Some(json!("fallback")));
        node.add_output_pin("label", "Label", "", VariableType::String)
            .set_options(optional.clone());
        node.add_output_pin("tags", "Tags", "", VariableType::String)
            .set_value_type(ValueType::Array)
            .set_options(optional.clone());
        node.add_output_pin("meta", "Meta", "", VariableType::Struct)
            .set_options(optional.clone());
        node.add_output_pin("anything", "Anything", "", VariableType::Generic)
            .set_options(optional.clone());
        node.add_output_pin("location", "Location", "", VariableType::Geometry)
            .set_options(optional)
            .schema = Some(marker(GeometryKind::Point).to_string());
        node
    }

    async fn assert_type_defaults(run: &InternalRun) {
        assert_eq!(pin_value(run, "label").await, Some(json!("")));
        assert_eq!(pin_value(run, "tags").await, Some(json!([])));
        assert_eq!(pin_value(run, "meta").await, Some(json!({})));
        assert_eq!(pin_value(run, "anything").await, Some(Value::Null));
        assert_eq!(pin_value(run, "location").await, Some(Value::Null));
    }

    async fn run_event(payload: Option<Value>) -> InternalRun {
        let (state, run) = build_run(payload).await;
        execute(state, run).await
    }

    async fn build_run(payload: Option<Value>) -> (Arc<FlowLikeState>, InternalRun) {
        build_run_with_node(payload, event_node(), Default::default()).await
    }

    async fn build_run_with_node(
        payload: Option<Value>,
        node: Node,
        refs: std::collections::HashMap<String, String>,
    ) -> (Arc<FlowLikeState>, InternalRun) {
        let state = Arc::new(FlowLikeState::new(
            FlowLikeConfig::new(),
            HTTPClient::new_without_refetch(),
        ));
        let logic: Arc<dyn NodeLogic> = Arc::new(GenericEventNode::new());
        let mut registry = FlowNodeRegistryInner::new(1);
        registry.insert(logic.get_node(), logic);
        state.node_registry.write().await.node_registry = Arc::new(registry);

        let mut board = Board::new_detached(Some("generic-event".to_string()), Path::default());
        board.refs = refs;
        board.nodes.insert(node.id.clone(), node);

        let run_payload = RunPayload {
            id: EVENT_ID.to_string(),
            payload,
            runtime_variables: None,
            filter_secrets: Some(true),
        };
        let intercom = BufferedInterComHandler::new(
            Arc::new(|_events| Box::pin(async { Ok(()) })),
            Some(100),
            Some(400),
            Some(false),
        );
        let run = InternalRun::new(
            "test-app",
            Arc::new(board),
            None,
            &state,
            &Profile::default(),
            &run_payload,
            false,
            intercom.into_callback(),
            None,
            None,
            std::collections::HashMap::new(),
        )
        .await
        .expect("build generic event run");
        (state, run)
    }

    async fn serve_upload(body: &'static [u8]) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind upload server");
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 2048];
            socket.read(&mut request).await.unwrap();
            let headers = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            socket.write_all(headers.as_bytes()).await.unwrap();
            socket.write_all(body).await.unwrap();
        });
        format!("http://{address}/picked.png")
    }

    fn uploaded_path(path: &str, url: &str) -> Value {
        json!({
            "path": path,
            "store_ref": flow_like_types::dispatch::REQUEST_FILES_STORE_REF,
            "cache_store_ref": null,
            "url": url,
            "name": "picked.png"
        })
    }

    async fn build_file_run(
        value_type: ValueType,
        value: Value,
        compact_schema: bool,
    ) -> (Arc<FlowLikeState>, InternalRun) {
        let mut node = event_node();
        let pin = node
            .add_output_pin("dateien", "Dateien", "", VariableType::Struct)
            .set_value_type(value_type)
            .set_schema::<FlowPath>();
        let mut refs = std::collections::HashMap::new();
        if compact_schema {
            refs.insert("123".to_string(), pin.schema.take().unwrap());
            pin.schema = Some("123".to_string());
        }
        let (state, run) = build_run_with_node(Some(json!({"Dateien": value})), node, refs).await;
        run.cache.write().await.insert(
            flow_like_types::dispatch::REQUEST_FILES_STORE_REF.to_string(),
            Arc::new(FlowLikeStore::Memory(Arc::new(
                flow_like_storage::object_store::memory::InMemory::new(),
            ))),
        );
        (state, run)
    }

    async fn execute(state: Arc<FlowLikeState>, mut run: InternalRun) -> InternalRun {
        run.execute(state).await;
        assert!(matches!(run.get_status().await, RunStatus::Success));
        run
    }

    async fn delegated_context(state: &Arc<FlowLikeState>, run: &InternalRun) -> ExecutionContext {
        let node = run.nodes.get(EVENT_ID).expect("event node");
        let mut context = ExecutionContext::new(
            run.nodes.clone(),
            &Arc::downgrade(&run.run),
            state,
            node,
            &run.variables,
            &run.cache,
            LogLevel::Debug,
            ExecutionStage::Dev,
            run.profile.clone(),
            run.callback.clone(),
            Arc::new(RwLock::new(Vec::new())),
            None,
            None,
            run.oauth_tokens.clone(),
            Some(run.channel.clone()),
        )
        .await;
        context.delegated = true;
        context.context_pin_overrides = Some(Default::default());
        context
    }

    async fn pin_value(run: &InternalRun, name: &str) -> Option<Value> {
        run.nodes
            .get(EVENT_ID)
            .expect("event node")
            .get_pin_by_name(name)
            .await
            .expect("output pin")
            .get_raw_value()
            .await
    }

    #[tokio::test]
    async fn scalar_form_upload_reaches_its_output_as_a_readable_flow_path() {
        let bytes = b"uploaded image bytes";
        let url = serve_upload(bytes).await;
        let path = "tmp/user/viewer/apps/test-app/2026/10/07/picked.png";
        let (state, run) = build_file_run(ValueType::Normal, uploaded_path(path, &url), true).await;
        let run = execute(state.clone(), run).await;
        let value = pin_value(&run, "dateien").await.unwrap();
        assert_eq!(
            value,
            json!({
                "path": path,
                "store_ref": flow_like_types::dispatch::REQUEST_FILES_STORE_REF,
                "cache_store_ref": null
            })
        );
        let output: FlowPath = flow_like_types::json::from_value(value).unwrap();
        let mut context = delegated_context(&state, &run).await;
        assert_eq!(output.get(&mut context, false).await.unwrap(), bytes);
        assert_eq!(pin_value(&run, "payload").await, Some(json!({})));
    }

    #[tokio::test]
    async fn array_and_set_form_uploads_materialize_each_file_in_order() {
        for value_type in [ValueType::Array, ValueType::HashSet] {
            let first_url = serve_upload(b"first image").await;
            let second_url = serve_upload(b"second image").await;
            let (state, run) = build_file_run(
                value_type,
                json!([
                    uploaded_path("tmp/user/viewer/apps/test-app/first.png", &first_url),
                    uploaded_path("tmp/user/viewer/apps/test-app/second.png", &second_url)
                ]),
                false,
            )
            .await;
            let run = execute(state.clone(), run).await;
            let value = pin_value(&run, "dateien").await.unwrap();
            assert!(
                value
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|value| value.get("url").is_none())
            );
            let paths: Vec<FlowPath> = flow_like_types::json::from_value(value).unwrap();
            let mut context = delegated_context(&state, &run).await;
            assert_eq!(paths.len(), 2);
            assert_eq!(
                paths[0].get(&mut context, false).await.unwrap(),
                b"first image"
            );
            assert_eq!(
                paths[1].get(&mut context, false).await.unwrap(),
                b"second image"
            );
        }
    }

    #[tokio::test]
    async fn unrelated_structs_and_flow_paths_without_source_urls_are_unchanged() {
        let unrelated = uploaded_path("unrelated", "file:///unrelated/picked.png");
        let mut node = event_node();
        node.add_output_pin("unrelated", "Unrelated", "", VariableType::Struct)
            .set_open_schema();
        node.add_output_pin("dateien", "Dateien", "", VariableType::Struct)
            .set_schema::<FlowPath>();
        let path = json!({"path": "existing.png", "store_ref": "other-run"});
        let (state, run) = build_run_with_node(
            Some(json!({"unrelated": unrelated, "dateien": path})),
            node,
            Default::default(),
        )
        .await;
        let run = execute(state, run).await;
        assert_eq!(pin_value(&run, "unrelated").await, Some(unrelated));
        assert_eq!(pin_value(&run, "dateien").await, Some(path));
    }

    #[tokio::test]
    async fn form_upload_urls_cannot_read_local_files() {
        let source = std::env::temp_dir().join(format!(
            "flow-like-form-source-{}.png",
            uuid::Uuid::new_v4()
        ));
        std::fs::write(&source, b"local file stays private").unwrap();
        let file_url = flow_like_types::reqwest::Url::from_file_path(&source).unwrap();
        let urls = [
            file_url.to_string(),
            format!("asset://localhost{}", file_url.path()),
            format!("http://asset.localhost{}", file_url.path()),
            format!("https://asset.localhost{}", file_url.path()),
            "https://asset.localhost/".to_string(),
            "data:image/png;base64,bG9jYWw=".to_string(),
        ];
        let (state, run) = build_file_run(ValueType::Normal, Value::Null, false).await;
        let mut context = delegated_context(&state, &run).await;
        let pin = context.get_pin_by_name("dateien").await.unwrap();
        let path = "tmp/user/viewer/apps/test-app/local.png";
        for url in urls {
            let error = materialize_pin_uploads(&mut context, &pin, &uploaded_path(path, &url))
                .await
                .unwrap_err();
            assert!(error.to_string().contains("HTTP or HTTPS upload URL"));
        }
        let target = FlowPath::new(
            path.to_string(),
            flow_like_types::dispatch::REQUEST_FILES_STORE_REF.to_string(),
            None,
        );
        assert!(target.get(&mut context, true).await.is_err());
        assert_eq!(std::fs::read(&source).unwrap(), b"local file stays private");
        std::fs::remove_file(source).unwrap();
    }

    #[tokio::test]
    async fn missing_optional_pins_receive_default_or_type_default_and_required_stay_unset() {
        let run = run_event(Some(json!({"extra": 1}))).await;

        assert_eq!(pin_value(&run, "name").await, Some(json!("anonymous")));
        assert_eq!(pin_value(&run, "count").await, Some(json!(0)));
        assert_eq!(pin_value(&run, "required").await, None);
        assert_type_defaults(&run).await;
        assert_eq!(pin_value(&run, "payload").await, Some(json!({"extra": 1})));
    }

    #[tokio::test]
    async fn supplied_values_win_over_defaults() {
        let run = run_event(Some(json!({
            "name": "felix",
            "count": 3,
            "required": "x",
            "tags": ["a"],
            "location": {"type": "Point", "coordinates": [13.405, 52.52]}
        })))
        .await;

        assert_eq!(pin_value(&run, "name").await, Some(json!("felix")));
        assert_eq!(pin_value(&run, "count").await, Some(json!(3)));
        assert_eq!(pin_value(&run, "required").await, Some(json!("x")));
        assert_eq!(pin_value(&run, "tags").await, Some(json!(["a"])));
        assert_eq!(
            pin_value(&run, "location").await,
            Some(json!({"type": "Point", "coordinates": [13.405, 52.52]}))
        );
        assert_eq!(pin_value(&run, "label").await, Some(json!("")));
        assert_eq!(pin_value(&run, "meta").await, Some(json!({})));
        assert_eq!(pin_value(&run, "anything").await, Some(Value::Null));
        assert_eq!(pin_value(&run, "payload").await, Some(json!({})));
    }

    #[tokio::test]
    async fn non_object_payload_fills_every_optional_pin() {
        let run = run_event(Some(json!("plain text"))).await;

        assert_eq!(pin_value(&run, "name").await, Some(json!("anonymous")));
        assert_eq!(pin_value(&run, "count").await, Some(json!(0)));
        assert_eq!(pin_value(&run, "required").await, None);
        assert_type_defaults(&run).await;
        assert_eq!(pin_value(&run, "payload").await, Some(json!("plain text")));
    }

    #[tokio::test]
    async fn absent_payload_fills_every_optional_pin() {
        let run = run_event(None).await;

        assert_eq!(pin_value(&run, "name").await, Some(json!("anonymous")));
        assert_eq!(pin_value(&run, "count").await, Some(json!(0)));
        assert_eq!(pin_value(&run, "required").await, None);
        assert_type_defaults(&run).await;
        assert_eq!(pin_value(&run, "payload").await, Some(json!({})));
    }

    #[tokio::test]
    async fn delegated_runs_fill_optional_pins_the_caller_left_unset() {
        let (state, run) = build_run(None).await;
        let mut context = delegated_context(&state, &run).await;
        let count = context.get_pin_by_name("count").await.expect("count pin");
        context
            .set_pin_ref_value(&count, json!(7))
            .await
            .expect("caller-provided argument");

        GenericEventNode::new()
            .run(&mut context)
            .await
            .expect("delegated run");

        assert_eq!(pin_value(&run, "name").await, Some(json!("anonymous")));
        assert_eq!(pin_value(&run, "count").await, Some(json!(7)));
        assert_eq!(pin_value(&run, "required").await, None);
        assert_type_defaults(&run).await;
        assert_eq!(pin_value(&run, "payload").await, None);
        assert_eq!(
            context
                .evaluate_pin::<Option<String>>("name")
                .await
                .unwrap(),
            Some("anonymous".to_string())
        );
    }

    #[tokio::test]
    async fn delegated_runs_type_default_optional_pins_without_a_default() {
        let (state, run) = build_run(None).await;
        let mut context = delegated_context(&state, &run).await;

        GenericEventNode::new()
            .run(&mut context)
            .await
            .expect("delegated run");

        assert_eq!(pin_value(&run, "count").await, Some(json!(0)));
        assert_type_defaults(&run).await;
        assert_eq!(
            context.evaluate_pin::<Option<i64>>("count").await.unwrap(),
            Some(0)
        );
        assert_eq!(
            context.evaluate_pin::<Vec<String>>("tags").await.unwrap(),
            Vec::<String>::new()
        );
        assert_eq!(
            context
                .evaluate_pin::<Option<String>>("label")
                .await
                .unwrap(),
            Some(String::new())
        );
    }
}
