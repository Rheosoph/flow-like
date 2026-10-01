use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use base64::Engine as _;
use flow_like_browser::event_log::EventCursor;
use flow_like_browser::script::{ELEMENT_KEY, SHADOW_KEY, ScriptArg, ScriptOptions, WINDOW_KEY};
use flow_like_browser::testing::{PageHarness, default_auto_reply};
use flow_like_browser::transport::memory::SentCommand;
use flow_like_browser::{BrowserError, Element, Frame};
use serde_json::{Value, json};

const IS_CONNECTED: &str = "function(){return this.isConnected}";
const WAIT: Duration = Duration::from_secs(5);

type Reply = Box<dyn Fn(&SentCommand) -> Option<Value> + Send + Sync>;

fn declaration(command: &SentCommand) -> &str {
    command.params["functionDeclaration"]
        .as_str()
        .unwrap_or_default()
}

fn is_call(command: &SentCommand) -> bool {
    command.method == "Runtime.callFunctionOn"
}

fn is_wrapper(command: &SentCommand) -> bool {
    is_call(command) && declaration(command).starts_with("function(script,args,w3c)")
}

fn script_body(command: &SentCommand) -> &str {
    command.params["arguments"][0]["value"]
        .as_str()
        .unwrap_or_default()
}

fn atom(command: &SentCommand) -> Option<&str> {
    declaration(command)
        .strip_prefix("function(a,b,c){return globalThis.__flowlike.")?
        .split('(')
        .next()
}

fn called_on(command: &SentCommand) -> &str {
    command.params["objectId"].as_str().unwrap_or_default()
}

fn by_value(value: Value) -> Value {
    json!({"result": {"type": "object", "value": value}})
}

fn thrown(description: &str) -> Value {
    json!({
        "result": {"type": "object"},
        "exceptionDetails": {"text": "Uncaught", "exception": {"type": "object", "description": description}},
    })
}

fn node(id: i64, loader: &str) -> Value {
    json!({"type": "node", "value": {"nodeType": 1, "backendNodeId": id, "loaderId": loader}})
}

fn deep(response: &str, nodes: &[Value]) -> Value {
    let mut list = vec![json!({"type": "string", "value": response})];
    list.extend(nodes.iter().cloned());
    json!({"result": {
        "type": "object",
        "subtype": "array",
        "objectId": "wrapper-result",
        "deepSerializedValue": {"type": "array", "value": list},
    }})
}

fn element_list(ids: &[i64], loader: &str) -> Value {
    let refs: Vec<Value> = (0..ids.len())
        .map(|index| json!({ELEMENT_KEY: index}))
        .collect();
    let nodes: Vec<Value> = ids.iter().map(|id| node(*id, loader)).collect();
    deep(&json!({"status": 0, "value": refs}).to_string(), &nodes)
}

fn minted(body: &str) -> Option<Value> {
    let mut parts = body.strip_prefix("mint:")?.splitn(2, ':');
    let loader = parts.next()?;
    let ids: Vec<i64> = parts
        .next()?
        .split(',')
        .filter_map(|id| id.parse().ok())
        .collect();
    Some(element_list(&ids, loader))
}

fn dom(command: &SentCommand) -> Option<Value> {
    match command.method.as_str() {
        "DOM.resolveNode" => Some(json!({"object": {
            "type": "object",
            "subtype": "node",
            "objectId": format!("node-{}", command.params["backendNodeId"]),
        }})),
        "Runtime.callFunctionOn" if declaration(command) == IS_CONNECTED => {
            Some(by_value(json!(true)))
        }
        "Runtime.callFunctionOn" if is_wrapper(command) => minted(script_body(command)),
        "Runtime.evaluate"
            if command.params["expression"]
                .as_str()
                .is_some_and(|expression| {
                    expression.starts_with("(function(){globalThis.__flowlike=")
                }) =>
        {
            Some(json!({"result": {"type": "undefined"}}))
        }
        _ => default_auto_reply(command),
    }
}

fn install(
    harness: &PageHarness,
    custom: impl Fn(&SentCommand) -> Option<Value> + Send + Sync + 'static,
) {
    let custom: Reply = Box::new(custom);
    harness
        .control
        .set_auto_reply(move |command| custom(command).or_else(|| dom(command)));
}

async fn mint(frame: &Frame, loader: &str, ids: &[i64]) -> Vec<Element> {
    let ids: Vec<String> = ids.iter().map(ToString::to_string).collect();
    let body = format!("mint:{loader}:{}", ids.join(","));
    frame
        .execute_script(&body, Vec::new(), ScriptOptions::USER)
        .await
        .unwrap()
        .elements()
        .unwrap()
}

async fn applied(harness: &PageHarness, cursor: EventCursor, method: &str) {
    let deadline = tokio::time::Instant::now() + WAIT;
    let seen = harness
        .connection
        .events()
        .wait_for(cursor, deadline, |event| &*event.method == method)
        .await
        .unwrap();
    assert!(seen.is_some(), "{method} was never processed");
}

async fn sent(
    harness: &PageHarness,
    what: &str,
    matches: impl Fn(&SentCommand) -> bool,
) -> SentCommand {
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        if let Some(command) = harness.control.commands_seen().into_iter().find(&matches) {
            return command;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "{what} was never sent"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn sent_after(harness: &PageHarness, count: usize, prefix: &str) -> Vec<SentCommand> {
    harness
        .control
        .commands_seen()
        .into_iter()
        .skip(count)
        .filter(|command| command.method.starts_with(prefix))
        .collect()
}

fn calls_where(harness: &PageHarness, matches: impl Fn(&SentCommand) -> bool) -> Vec<SentCommand> {
    harness
        .control
        .commands_seen()
        .into_iter()
        .filter(|command| matches(command))
        .collect()
}

fn failure<T>(result: flow_like_browser::Result<T>) -> BrowserError {
    match result {
        Ok(_) => panic!("the operation succeeded, but an error was expected"),
        Err(error) => error,
    }
}

fn golden_rows() -> Vec<Value> {
    let text = include_str!("fixtures/parity/golden/b_script_plain.json");
    let text = flow_like_browser::protocol::sanitize_lone_surrogates(text)
        .unwrap_or_else(|| text.to_owned());
    let golden: Value = serde_json::from_str(&text).unwrap();
    golden["rows"].as_array().unwrap().clone()
}

fn golden_node(summary: &str) -> Value {
    match summary.rsplit_once(':') {
        Some(("node:window", _)) => json!({"type": "window", "value": {"context": "T1"}}),
        Some((_, id)) => node(id.parse().unwrap(), "L1"),
        None => panic!("unexpected node summary {summary}"),
    }
}

fn golden_reply(row: &Value) -> Value {
    let port = &row["cdp"]["chromedriver_port"];
    let response = if row["name"] == "lone_surrogate" {
        r#"{"status":0,"value":"a\ud83d"}"#.to_owned()
    } else if port.get("parsed").is_some() {
        port["parsed"].to_string()
    } else {
        let message = row["wd"]["message"].as_str().unwrap();
        json!({"status": 17, "value": message.trim_start_matches("javascript error: ")}).to_string()
    };
    let nodes: Vec<Value> = port["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(golden_node)
        .collect();
    deep(&response, &nodes)
}

fn normalized(value: &Value) -> Value {
    match value {
        Value::String(text)
            if text.starts_with("f.") && text.contains(".d.") && text.contains(".e.") =>
        {
            json!("<element>")
        }
        Value::Array(items) => Value::Array(items.iter().map(normalized).collect()),
        Value::Object(object) if object.contains_key(WINDOW_KEY) => json!({WINDOW_KEY: "<window>"}),
        Value::Object(object) => Value::Object(
            object
                .iter()
                .map(|(key, item)| (key.clone(), normalized(item)))
                .collect(),
        ),
        other => other.clone(),
    }
}

async fn check_golden_row(frame: &Frame, row: &Value, argument: &Element) {
    let name = row["name"].as_str().unwrap();
    let body = row["body"].as_str().unwrap();
    let args = if name == "element_argument" {
        vec![
            ScriptArg::Element(argument.clone()),
            ScriptArg::Json(json!(5)),
        ]
    } else {
        Vec::new()
    };
    let result = frame.execute_script(body, args, ScriptOptions::USER).await;
    let wd = &row["wd"];
    match (wd.get("ok"), wd["error"].as_str()) {
        (Some(expected), _) => {
            let value = result.unwrap_or_else(|error| panic!("{name}: {error}"));
            assert_eq!(normalized(value.json()), normalized(expected), "{name}");
        }
        (None, Some("javascript error")) => {
            let error = result.err().unwrap_or_else(|| panic!("{name} succeeded"));
            assert!(
                matches!(error, BrowserError::Javascript { .. }),
                "{name}: {error}"
            );
            assert_eq!(error.to_string(), wd["message"].as_str().unwrap(), "{name}");
        }
        (None, Some("stale element reference")) => {
            assert!(matches!(result, Err(BrowserError::StaleElement)), "{name}");
        }
        (None, Some("unknown error")) => {
            assert_eq!(name, "lone_surrogate");
            assert_eq!(result.unwrap().json(), &json!("a\u{FFFD}"));
        }
        other => panic!("{name}: unexpected golden shape {other:?}"),
    }
}

async fn golden_harness() -> (PageHarness, Vec<Value>) {
    let rows = golden_rows();
    let replies: HashMap<String, Value> = rows
        .iter()
        .map(|row| (row["body"].as_str().unwrap().to_owned(), golden_reply(row)))
        .collect();
    let harness = PageHarness::new().await;
    install(&harness, move |command| {
        is_wrapper(command)
            .then(|| replies.get(script_body(command)).cloned())
            .flatten()
    });
    (harness, rows)
}

async fn user_script(frame: &Frame, body: &str) -> flow_like_browser::script::ScriptValue {
    frame
        .execute_script(body, Vec::new(), ScriptOptions::USER)
        .await
        .unwrap()
}

#[tokio::test]
async fn script_results_match_the_chromedriver_golden() {
    let (harness, rows) = golden_harness().await;
    assert_eq!(rows.len(), 51);
    let frame = harness.page.main_frame();
    let argument = mint(&frame, "L1", &[15]).await.remove(0);
    for row in &rows {
        check_golden_row(&frame, row, &argument).await;
    }
}

#[tokio::test]
async fn golden_node_results_carry_our_element_ids() {
    let (harness, _) = golden_harness().await;
    let frame = harness.page.main_frame();
    let body = user_script(&frame, "return document.body").await;
    assert_eq!(body.json(), &json!({ELEMENT_KEY: "f.T1.d.L1.e.8"}));
    assert_eq!(body.element().unwrap().backend_node_id(), 8);
    let window = user_script(&frame, "return window").await;
    assert_eq!(window.json(), &json!({WINDOW_KEY: "T1"}));
    assert!(matches!(
        window.element(),
        Err(BrowserError::NotFound { .. })
    ));
    let shadow = user_script(
        &frame,
        "return document.querySelector('#t_shadow_host').shadowRoot",
    )
    .await;
    assert_eq!(shadow.json(), &json!({SHADOW_KEY: "f.T1.d.L1.e.8"}));
    let list = user_script(&frame, "return document.querySelectorAll('li')").await;
    let ids: Vec<i64> = list
        .elements()
        .unwrap()
        .iter()
        .map(Element::backend_node_id)
        .collect();
    assert_eq!(ids, vec![8, 9]);
}

#[tokio::test]
async fn scripts_run_the_wrapper_in_the_requested_world() {
    let harness = PageHarness::new().await;
    install(&harness, |command| {
        is_wrapper(command).then(|| deep(r#"{"status":0,"value":1}"#, &[]))
    });
    let frame = harness.page.main_frame();
    let value = frame
        .execute_script(
            "return 1",
            vec![ScriptArg::Json(json!("x"))],
            ScriptOptions::USER,
        )
        .await
        .unwrap();
    assert_eq!(value.json(), &json!(1));
    let call = sent(&harness, "the user script", is_wrapper).await;
    assert_eq!(call.session.as_deref(), Some("S1"));
    assert_eq!(call.params["uniqueContextId"], "S1-main");
    assert_eq!(call.params["awaitPromise"], true);
    assert_eq!(
        call.params["serializationOptions"],
        json!({"serialization": "deep"})
    );
    assert_eq!(
        call.params["arguments"],
        json!([{"value": "return 1"}, {"value": ["x"]}, {"value": true}])
    );
    let group = call.params["objectGroup"].as_str().unwrap().to_owned();
    assert!(group.starts_with("flowlike-call-"));
    sent(&harness, "the object group release", |command| {
        command.method == "Runtime.releaseObjectGroup" && command.params["objectGroup"] == group
    })
    .await;
    frame
        .execute_script("return 1", Vec::new(), ScriptOptions::INTERNAL)
        .await
        .unwrap();
    let internal = calls_where(&harness, is_wrapper);
    assert_eq!(
        internal.last().unwrap().params["uniqueContextId"],
        "S1-util"
    );
}

#[tokio::test]
async fn element_arguments_become_node_references() {
    let harness = PageHarness::new().await;
    install(&harness, |command| {
        (is_wrapper(command) && !script_body(command).starts_with("mint:"))
            .then(|| deep(r#"{"status":0,"value":"t_plain:5"}"#, &[]))
    });
    let frame = harness.page.main_frame();
    let element = mint(&frame, "L1", &[8]).await.remove(0);
    let body = "return arguments[0].id + ':' + arguments[1]";
    let args = vec![ScriptArg::Element(element), ScriptArg::Json(json!(5))];
    let value = frame
        .execute_script(body, args, ScriptOptions::USER)
        .await
        .unwrap();
    assert_eq!(value.json(), &json!("t_plain:5"));
    let call = calls_where(&harness, |command| {
        is_wrapper(command) && script_body(command) == body
    })
    .remove(0);
    assert_eq!(
        call.params["arguments"][1]["value"],
        json!([{ELEMENT_KEY: 0}, 5])
    );
    assert_eq!(call.params["arguments"][3], json!({"objectId": "node-8"}));
    let resolved = calls_where(&harness, |command| command.method == "DOM.resolveNode").remove(0);
    assert_eq!(resolved.params["executionContextId"], 1);
}

#[tokio::test]
async fn elements_of_another_frame_are_rejected() {
    let harness = PageHarness::new().await;
    install(&harness, |_| None);
    harness.attach_child("S2", "F2", "T1", "http://127.0.0.1/child", "L2");
    let child = harness.page.frame(&"F2".into()).unwrap();
    let element = mint(&child, "L2", &[4]).await.remove(0);
    assert_eq!(element.web_element_id(), "f.F2.d.L2.e.4");
    let main = harness.page.main_frame();
    let error = failure(
        main.execute_script(
            "return 1",
            vec![ScriptArg::Element(element.clone())],
            ScriptOptions::USER,
        )
        .await,
    );
    assert_eq!(
        error.to_string(),
        "invalid argument: Element belongs to another frame"
    );
    let error = failure(main.find_css("p", Some(&element)).await);
    assert!(
        matches!(error, BrowserError::InvalidArgument { .. }),
        "{error}"
    );
}

#[tokio::test]
async fn a_new_document_makes_elements_stale_before_any_dom_command() {
    let harness = PageHarness::new().await;
    install(&harness, |_| None);
    let element = mint(&harness.page.main_frame(), "L1", &[8]).await.remove(0);
    let cursor = harness.connection.events().cursor();
    harness.emit(
        "Page.frameNavigated",
        json!({"frame": {"id": "T1", "loaderId": "L2", "url": "http://127.0.0.1/next"}, "type": "Navigation"}),
    );
    applied(&harness, cursor, "Page.frameNavigated").await;
    let count = harness.control.commands_seen().len();
    assert!(matches!(
        element.text().await,
        Err(BrowserError::StaleElement)
    ));
    assert!(matches!(
        element.clear().await,
        Err(BrowserError::StaleElement)
    ));
    assert!(sent_after(&harness, count, "DOM.").is_empty());
}

#[tokio::test]
async fn an_oopif_element_is_stale_after_a_swap_back_without_dom_traffic() {
    let harness = PageHarness::new().await;
    install(&harness, |_| None);
    harness.attach_child("S2", "F2", "T1", "http://127.0.0.1/child", "L2");
    let child = harness.page.frame(&"F2".into()).unwrap();
    let element = mint(&child, "L2", &[4]).await.remove(0);
    let cursor = harness.connection.events().cursor();
    harness.emit(
        "Page.frameAttached",
        json!({"frameId": "F2", "parentFrameId": "T1"}),
    );
    harness.emit(
        "Page.frameNavigated",
        json!({"frame": {"id": "F2", "parentId": "T1", "loaderId": "L3", "url": "http://127.0.0.1/child"}, "type": "Navigation"}),
    );
    applied(&harness, cursor, "Page.frameNavigated").await;
    let count = harness.control.commands_seen().len();
    assert!(matches!(
        element.is_displayed().await,
        Err(BrowserError::StaleElement)
    ));
    assert!(sent_after(&harness, count, "DOM.").is_empty());
}

async fn navigate_during_the_script(options: ScriptOptions) -> flow_like_browser::Result<String> {
    let mut harness = PageHarness::new().await;
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    install(&harness, move |command| {
        let body = script_body(command) == "return document.body";
        if !is_wrapper(command) || !body {
            return None;
        }
        let call = counter.fetch_add(1, Ordering::SeqCst);
        (call > 0).then(|| element_list(&[8], "L2"))
    });
    let frame = harness.page.main_frame();
    let script = tokio::spawn(async move {
        frame
            .execute_script("return document.body", Vec::new(), options)
            .await
            .map(|value| value.elements().unwrap()[0].web_element_id())
    });
    let held = harness
        .control
        .wait_for("Runtime.callFunctionOn", Some("S1"))
        .await;
    harness.emit(
        "Page.frameNavigated",
        json!({"frame": {"id": "T1", "loaderId": "L2", "url": "http://127.0.0.1/next"}, "type": "Navigation"}),
    );
    harness.control.reply(&held, element_list(&[8], "L1"));
    tokio::time::timeout(WAIT, script).await.unwrap().unwrap()
}

#[tokio::test]
async fn a_probe_interrupted_by_a_new_document_retries_with_the_new_loader() {
    let id = navigate_during_the_script(ScriptOptions::PROBE)
        .await
        .unwrap();
    assert_eq!(id, "f.T1.d.L2.e.8");
}

#[tokio::test]
async fn a_user_script_interrupted_by_a_new_document_is_not_retried() {
    let error = navigate_during_the_script(ScriptOptions::USER)
        .await
        .unwrap_err();
    assert!(
        matches!(error, BrowserError::NavigationInterrupted),
        "{error}"
    );
}

#[tokio::test]
async fn atoms_install_once_per_context_unique_id() {
    let harness = PageHarness::new().await;
    install(&harness, |command| {
        (atom(command) == Some("getText")).then(|| by_value(json!("hello")))
    });
    let element = mint(&harness.page.main_frame(), "L1", &[8]).await.remove(0);
    assert_eq!(element.text().await.unwrap(), "hello");
    assert_eq!(element.text().await.unwrap(), "hello");
    let cursor = harness.connection.events().cursor();
    harness.emit(
        "Runtime.executionContextDestroyed",
        json!({"executionContextId": 2, "executionContextUniqueId": "S1-util"}),
    );
    harness.context_created("S1", "T1", 7, "S1-util-2", true);
    applied(&harness, cursor, "Runtime.executionContextCreated").await;
    assert_eq!(element.text().await.unwrap(), "hello");
    let installs: Vec<i64> = calls_where(&harness, |command| {
        command.method == "Runtime.evaluate"
            && command.params["expression"]
                .as_str()
                .is_some_and(|expression| {
                    expression.starts_with("(function(){globalThis.__flowlike=")
                })
    })
    .iter()
    .filter_map(|command| command.params["contextId"].as_i64())
    .collect();
    assert_eq!(installs, vec![2, 7]);
}

#[tokio::test]
async fn getters_use_the_atoms_and_the_right_world() {
    let harness = PageHarness::new().await;
    install(&harness, |command| {
        let reply = match atom(command) {
            Some("isDisplayed") => json!(true),
            Some("isEnabled") => json!(false),
            Some("isSelected") => json!(true),
            Some("getLocation") => json!({"x": 8.5, "y": 16}),
            Some("getSize") => json!({"width": 1169, "height": 21}),
            _ => match declaration(command) {
                "function(){return this.tagName.toLowerCase()}" => json!("input"),
                "function(name){return this[name]}" => {
                    json!(format!(
                        "<{}>",
                        command.params["arguments"][0]["value"].as_str()?
                    ))
                }
                _ => return None,
            },
        };
        is_call(command).then(|| by_value(reply))
    });
    let element = mint(&harness.page.main_frame(), "L1", &[8]).await.remove(0);
    assert!(element.is_displayed().await.unwrap());
    assert!(!element.is_enabled().await.unwrap());
    assert!(element.is_selected().await.unwrap());
    let rect = element.rect().await.unwrap();
    assert_eq!(
        (rect.x, rect.y, rect.width, rect.height),
        (8.5, 16.0, 1169.0, 21.0)
    );
    assert_eq!(element.tag_name().await.unwrap(), "input");
    assert_eq!(element.outer_html().await.unwrap(), "<outerHTML>");
    assert_eq!(element.inner_html().await.unwrap(), "<innerHTML>");
    assert_eq!(element.property("value").await.unwrap(), json!("<value>"));
    let displayed = calls_where(&harness, |command| atom(command) == Some("isDisplayed")).remove(0);
    assert_eq!(displayed.params["arguments"], json!([{"value": false}]));
    let contexts: Vec<i64> = calls_where(&harness, |command| command.method == "DOM.resolveNode")
        .iter()
        .filter_map(|command| command.params["executionContextId"].as_i64())
        .collect();
    assert_eq!(
        contexts.last(),
        Some(&1),
        "properties are read in the main world"
    );
    assert!(contexts.contains(&2), "atoms run in the utility world");
}

#[tokio::test]
async fn boolean_attributes_report_true_or_nothing() {
    let harness = PageHarness::new().await;
    install(&harness, |command| {
        let arguments = &command.params["arguments"];
        let getter = declaration(command).starts_with("function(name,boolean)");
        (is_call(command) && getter).then(|| {
            match (
                arguments[0]["value"].as_str(),
                arguments[1]["value"].as_bool(),
            ) {
                (Some("Checked"), Some(true)) => by_value(json!("true")),
                (Some("disabled"), Some(true)) => by_value(Value::Null),
                (Some(name), Some(false)) => by_value(json!(format!("value of {name}"))),
                _ => by_value(json!("unexpected")),
            }
        })
    });
    let element = mint(&harness.page.main_frame(), "L1", &[8]).await.remove(0);
    assert_eq!(
        element.attribute("Checked").await.unwrap().as_deref(),
        Some("true")
    );
    assert_eq!(element.attribute("disabled").await.unwrap(), None);
    assert_eq!(
        element.attribute("href").await.unwrap().as_deref(),
        Some("value of href")
    );
}

fn clear_replies(
    editable: bool,
    displayed: bool,
    cleared: Value,
) -> impl Fn(&SentCommand) -> Option<Value> + Send + Sync + 'static {
    move |command| {
        if declaration(command).starts_with("function(inputTypes)") {
            return Some(by_value(json!(editable)));
        }
        match atom(command) {
            Some("isDisplayed") => Some(by_value(json!(displayed))),
            Some("clear") => Some(cleared.clone()),
            _ => None,
        }
    }
}

async fn clear_harness(editable: bool, displayed: bool, cleared: Value) -> (PageHarness, Element) {
    let harness = PageHarness::new().await;
    install(&harness, clear_replies(editable, displayed, cleared));
    let element = mint(&harness.page.main_frame(), "L1", &[8]).await.remove(0);
    (harness, element)
}

#[tokio::test]
async fn clear_rejects_fields_that_are_not_user_editable() {
    let (harness, element) = clear_harness(false, true, by_value(Value::Null)).await;
    let error = element.clear().await.unwrap_err();
    assert_eq!(
        error.to_string(),
        "invalid element state: Element must be user-editable in order to clear it"
    );
    let check = calls_where(&harness, |command| {
        declaration(command).starts_with("function(inputTypes)")
    })
    .remove(0);
    let types = check.params["arguments"][0]["value"].as_array().unwrap();
    assert_eq!(types.len(), 15);
    assert!(calls_where(&harness, |command| atom(command).is_some()).is_empty());
}

#[tokio::test]
async fn clear_rejects_hidden_fields() {
    let (_harness, element) = clear_harness(true, false, by_value(Value::Null)).await;
    let error = element.clear().await.unwrap_err();
    assert!(
        matches!(error, BrowserError::NotInteractable { .. }),
        "{error}"
    );
}

#[tokio::test]
async fn clear_reports_an_atom_refusal_as_invalid_element_state() {
    let refused = thrown(
        "InvalidElementStateError: Element must be user-editable in order to clear it.\n    at clear",
    );
    let (_harness, element) = clear_harness(true, true, refused).await;
    let error = element.clear().await.unwrap_err();
    assert!(
        matches!(&error, BrowserError::InvalidElementState { message } if message == "Element must be user-editable in order to clear it."),
        "{error}"
    );
}

#[tokio::test]
async fn clear_runs_the_checks_before_the_atom() {
    let (harness, element) = clear_harness(true, true, by_value(Value::Null)).await;
    element.clear().await.unwrap();
    let steps: Vec<String> = calls_where(&harness, |command| {
        is_call(command) && declaration(command) != IS_CONNECTED && !is_wrapper(command)
    })
    .iter()
    .map(|command| atom(command).unwrap_or("pre-check").to_owned())
    .collect();
    assert_eq!(steps, vec!["pre-check", "isDisplayed", "clear"]);
    let displayed = calls_where(&harness, |command| atom(command) == Some("isDisplayed")).remove(0);
    assert_eq!(displayed.params["arguments"], json!([{"value": true}]));
    let resolves = calls_where(&harness, |command| command.method == "DOM.resolveNode");
    assert_eq!(resolves.len(), 1);
}

async fn open_alert(harness: &PageHarness) {
    harness.emit(
        "Page.javascriptDialogOpening",
        json!({"url": "http://127.0.0.1/", "message": "hi", "type": "alert", "hasBrowserHandler": false, "defaultPrompt": ""}),
    );
    let deadline = tokio::time::Instant::now() + WAIT;
    while harness.page.pending_dialog().is_none() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the dialog never opened"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn a_dialog_before_the_clear_commits_skips_the_atom() {
    let mut harness = PageHarness::new().await;
    install(&harness, |command| {
        if declaration(command).starts_with("function(inputTypes)") {
            return Some(by_value(json!(true)));
        }
        (atom(command) == Some("clear")).then(|| by_value(Value::Null))
    });
    let element = mint(&harness.page.main_frame(), "L1", &[8]).await.remove(0);
    let clearing = tokio::spawn(async move { element.clear().await });
    let held = harness
        .control
        .wait_for("Runtime.callFunctionOn", Some("S1"))
        .await;
    assert_eq!(atom(&held), Some("isDisplayed"));
    open_alert(&harness).await;
    harness.control.reply(&held, by_value(json!(true)));
    tokio::time::timeout(WAIT, clearing)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(calls_where(&harness, |command| atom(command) == Some("clear")).is_empty());
}

struct Uploads {
    harness: PageHarness,
    element: Element,
    multiple: Arc<Mutex<bool>>,
    directory: tempfile::TempDir,
}

impl Uploads {
    async fn new() -> Uploads {
        let directory = tempfile::tempdir().unwrap();
        for name in ["a.txt", "b.txt"] {
            std::fs::write(directory.path().join(name), name).unwrap();
        }
        std::fs::create_dir(directory.path().join("sub")).unwrap();
        let multiple = Arc::new(Mutex::new(false));
        let harness = PageHarness::new().await;
        let flag = multiple.clone();
        install(&harness, move |command| {
            if declaration(command) == "function(){return this.multiple===true}" {
                let multiple = *flag.lock().unwrap_or_else(PoisonError::into_inner);
                return Some(by_value(json!(multiple)));
            }
            (command.method == "DOM.setFileInputFiles").then(|| json!({}))
        });
        let element = mint(&harness.page.main_frame(), "L1", &[8]).await.remove(0);
        Uploads {
            harness,
            element,
            multiple,
            directory,
        }
    }

    fn path(&self, name: &str) -> std::path::PathBuf {
        self.directory.path().join(name)
    }

    fn uploads(&self) -> Vec<SentCommand> {
        calls_where(&self.harness, |command| {
            command.method == "DOM.setFileInputFiles"
        })
    }
}

#[tokio::test]
async fn set_files_sends_canonical_existing_paths() {
    let uploads = Uploads::new().await;
    let indirect = uploads.path("sub").join("..").join("a.txt");
    uploads.element.set_files(&[indirect]).await.unwrap();
    let upload = uploads.uploads().remove(0);
    let canonical = std::fs::canonicalize(uploads.path("a.txt")).unwrap();
    assert_eq!(upload.params["files"], json!([canonical.to_string_lossy()]));
    assert_eq!(upload.params["backendNodeId"], 8);
    assert_eq!(upload.session.as_deref(), Some("S1"));
}

#[tokio::test]
async fn set_files_checks_existence_and_the_multiple_attribute() {
    let uploads = Uploads::new().await;
    let missing = uploads.path("missing.txt");
    let error = uploads
        .element
        .set_files(std::slice::from_ref(&missing))
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        format!("invalid argument: File not found: {}", missing.display())
    );
    let both = [uploads.path("a.txt"), uploads.path("b.txt")];
    let error = uploads.element.set_files(&both).await.unwrap_err();
    assert_eq!(
        error.to_string(),
        "invalid argument: the element can not hold multiple files"
    );
    assert!(uploads.uploads().is_empty());
    *uploads
        .multiple
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = true;
    uploads.element.set_files(&both).await.unwrap();
    let files = &uploads.uploads().remove(0).params["files"];
    assert_eq!(files.as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn an_upload_answered_after_a_new_document_is_not_reported_as_done() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("a.txt");
    std::fs::write(&file, "a").unwrap();
    let mut harness = PageHarness::new().await;
    install(&harness, |_| None);
    let element = mint(&harness.page.main_frame(), "L1", &[8]).await.remove(0);
    let uploading = tokio::spawn(async move { element.set_files(&[file]).await });
    let held = harness
        .control
        .wait_for("DOM.setFileInputFiles", Some("S1"))
        .await;
    let cursor = harness.connection.events().cursor();
    harness.emit(
        "Page.frameNavigated",
        json!({"frame": {"id": "T1", "loaderId": "L2", "url": "http://127.0.0.1/next"}, "type": "Navigation"}),
    );
    applied(&harness, cursor, "Page.frameNavigated").await;
    harness.control.reply(&held, json!({}));
    let error = failure(
        tokio::time::timeout(WAIT, uploading)
            .await
            .unwrap()
            .unwrap(),
    );
    assert!(
        matches!(&error, BrowserError::LoaderChanged { method } if method == "DOM.setFileInputFiles"),
        "{error}"
    );
}

#[derive(Default)]
struct SelectDom {
    options: Vec<i64>,
    selected: Vec<i64>,
    multiple: bool,
    clicked: Vec<(i64, Value)>,
}

fn object_node(command: &SentCommand) -> Option<i64> {
    called_on(command).strip_prefix("node-")?.parse().ok()
}

fn select_array_reply(dom: &SelectDom, command: &SentCommand) -> Option<Value> {
    match command.method.as_str() {
        "Runtime.getProperties" => {
            let mut properties: Vec<Value> = dom
                .options
                .iter()
                .enumerate()
                .rev()
                .map(|(index, id)| json!({"name": index.to_string(), "value": {"type": "object", "objectId": format!("option-{id}")}}))
                .collect();
            properties.push(
                json!({"name": "length", "value": {"type": "number", "value": dom.options.len()}}),
            );
            Some(json!({"result": properties}))
        }
        "DOM.describeNode" => {
            let object = command.params["objectId"].as_str()?;
            let id: i64 = object.strip_prefix("option-")?.parse().ok()?;
            Some(json!({"node": {"nodeId": 0, "backendNodeId": id}}))
        }
        _ if declaration(command).starts_with("function(selector)") => {
            Some(json!({"result": {"type": "object", "subtype": "array", "objectId": "options"}}))
        }
        _ => None,
    }
}

fn select_function_reply(dom: &mut SelectDom, command: &SentCommand) -> Option<Value> {
    let node = object_node(command)?;
    let decl = declaration(command);
    let value = match atom(command) {
        Some("isSelected") => json!(dom.selected.contains(&node)),
        Some("click") => {
            dom.clicked
                .push((node, command.params["arguments"].clone()));
            Value::Null
        }
        _ if decl == "function(){return this.multiple===true}" => json!(dom.multiple),
        _ if decl.contains("isOptionElementToggleable(this)") => json!(dom.multiple),
        _ => return None,
    };
    Some(by_value(value))
}

fn select_reply(dom: &Mutex<SelectDom>, command: &SentCommand) -> Option<Value> {
    let mut dom = dom.lock().unwrap_or_else(PoisonError::into_inner);
    select_array_reply(&dom, command).or_else(|| select_function_reply(&mut dom, command))
}

async fn select_harness(dom: SelectDom) -> (PageHarness, Arc<Mutex<SelectDom>>, Element) {
    let harness = PageHarness::new().await;
    let dom = Arc::new(Mutex::new(dom));
    let shared = dom.clone();
    install(&harness, move |command| select_reply(&shared, command));
    let select = mint(&harness.page.main_frame(), "L1", &[20])
        .await
        .remove(0);
    (harness, dom, select)
}

fn clicked(dom: &Mutex<SelectDom>) -> Vec<(i64, Value)> {
    dom.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clicked
        .clone()
}

#[tokio::test]
async fn select_by_value_reports_a_missing_option() {
    let (harness, dom, select) = select_harness(SelectDom::default()).await;
    let error = select.select_by_value("b\"q").await.unwrap_err();
    assert_eq!(
        error.to_string(),
        "Could not find an option with value 'b\"q'"
    );
    let find = calls_where(&harness, |command| {
        declaration(command).starts_with("function(selector)")
    })
    .remove(0);
    assert_eq!(
        find.params["arguments"][0]["value"],
        "option[value=\"b\\\"q\"]"
    );
    assert_eq!(find.params["objectId"], "node-20");
    assert!(clicked(&dom).is_empty());
}

#[tokio::test]
async fn a_single_select_stops_at_the_first_matching_option() {
    let dom = SelectDom {
        options: vec![21, 22],
        ..SelectDom::default()
    };
    let (_harness, dom, select) = select_harness(dom).await;
    select.select_by_value("b").await.unwrap();
    assert_eq!(clicked(&dom), vec![(21, json!([{"value": true}]))]);
}

#[tokio::test]
async fn an_already_selected_first_option_is_left_alone() {
    let dom = SelectDom {
        options: vec![21, 22],
        selected: vec![21],
        ..SelectDom::default()
    };
    let (_harness, dom, select) = select_harness(dom).await;
    select.select_by_value("b").await.unwrap();
    assert!(clicked(&dom).is_empty());
}

#[tokio::test]
async fn a_multiple_select_selects_every_matching_option() {
    let dom = SelectDom {
        options: vec![21, 22, 23],
        selected: vec![22],
        multiple: true,
        ..SelectDom::default()
    };
    let (_harness, dom, select) = select_harness(dom).await;
    select.select_by_value("b").await.unwrap();
    let ids: Vec<i64> = clicked(&dom).into_iter().map(|(id, _)| id).collect();
    assert_eq!(ids, vec![21, 23]);
}

#[tokio::test]
async fn a_dialog_after_the_first_option_ends_a_multiple_select() {
    let mut harness = PageHarness::new().await;
    let dom = Arc::new(Mutex::new(SelectDom {
        options: vec![21, 22],
        multiple: true,
        ..SelectDom::default()
    }));
    let shared = dom.clone();
    install(&harness, move |command| {
        let blocked = object_node(command) == Some(22) && atom(command) == Some("isSelected");
        (!blocked).then(|| select_reply(&shared, command)).flatten()
    });
    let select = mint(&harness.page.main_frame(), "L1", &[20])
        .await
        .remove(0);
    let selecting = tokio::spawn(async move { select.select_by_value("b").await });
    let held = harness
        .control
        .wait_for("Runtime.callFunctionOn", Some("S1"))
        .await;
    assert_eq!(
        (object_node(&held), atom(&held)),
        (Some(22), Some("isSelected"))
    );
    open_alert(&harness).await;
    tokio::time::timeout(WAIT, selecting)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let ids: Vec<i64> = clicked(&dom).into_iter().map(|(id, _)| id).collect();
    assert_eq!(ids, vec![21]);
}

#[tokio::test]
async fn select_by_exact_text_matches_the_option_text_node() {
    let dom = SelectDom {
        options: vec![21],
        ..SelectDom::default()
    };
    let (harness, dom, select) = select_harness(dom).await;
    select.select_by_exact_text("Two").await.unwrap();
    select.select_by_exact_text("it's \"x\"").await.unwrap();
    let queries: Vec<Value> = calls_where(&harness, |command| {
        declaration(command).starts_with("function(selector)")
    })
    .iter()
    .map(|command| command.params["arguments"][0]["value"].clone())
    .collect();
    assert_eq!(
        queries,
        vec![
            json!(".//option[text() = \"Two\"]"),
            json!(".//option[text() = concat(\"it's \", '\"', \"x\", '\"', \"\")]"),
        ]
    );
    assert!(
        declaration(
            &calls_where(&harness, |command| declaration(command)
                .starts_with("function(selector)"))[0]
        )
        .contains("document.evaluate")
    );
    assert_eq!(clicked(&dom).len(), 2);
    let error = SelectDom::default();
    let (_harness, _dom, select) = select_harness(error).await;
    assert_eq!(
        select
            .select_by_exact_text("Two")
            .await
            .unwrap_err()
            .to_string(),
        "Could not find an option with text 'Two'"
    );
}

#[tokio::test]
async fn find_css_expands_the_result_array_in_index_order() {
    let dom = SelectDom {
        options: vec![31, 32],
        ..SelectDom::default()
    };
    let (harness, _dom, scope) = select_harness(dom).await;
    let frame = harness.page.main_frame();
    let found = frame.find_css("p", None).await.unwrap();
    let ids: Vec<String> = found.iter().map(Element::web_element_id).collect();
    assert_eq!(ids, vec!["f.T1.d.L1.e.31", "f.T1.d.L1.e.32"]);
    let find = calls_where(&harness, |command| {
        declaration(command).starts_with("function(selector)")
    })
    .remove(0);
    assert_eq!(find.params["uniqueContextId"], "S1-util");
    assert!(declaration(&find).contains("querySelectorAll"));
    let group = find.params["objectGroup"].as_str().unwrap().to_owned();
    sent(&harness, "the search group release", |command| {
        command.method == "Runtime.releaseObjectGroup" && command.params["objectGroup"] == group
    })
    .await;
    let scoped = frame.find_xpath(".//p", Some(&scope)).await.unwrap();
    assert_eq!(scoped.len(), 2);
    let find = calls_where(&harness, |command| {
        declaration(command).starts_with("function(selector)")
    })
    .pop()
    .unwrap();
    assert_eq!(find.params["objectId"], "node-20");
    assert!(find.params.get("uniqueContextId").is_none());
}

#[tokio::test]
async fn invalid_selectors_are_invalid_arguments() {
    let harness = PageHarness::new().await;
    install(&harness, |command| {
        declaration(command).starts_with("function(selector)").then(|| {
            json!({
                "result": {"type": "string", "value": "invalid selector: The result of the xpath expression is not an element"},
                "exceptionDetails": {"text": "Uncaught", "exception": {"type": "string", "value": "invalid selector: The result of the xpath expression is not an element"}},
            })
        })
    });
    let error = failure(harness.page.main_frame().find_xpath("//@id", None).await);
    assert_eq!(
        error.to_string(),
        "invalid argument: invalid selector: The result of the xpath expression is not an element"
    );
}

#[tokio::test(start_paused = true)]
async fn a_script_that_never_settles_times_out_after_30_seconds() {
    let harness = PageHarness::new().await;
    install(&harness, |_| None);
    let started = tokio::time::Instant::now();
    let error = failure(
        harness
            .page
            .main_frame()
            .execute_script(
                "await new Promise(() => {})",
                Vec::new(),
                ScriptOptions::USER,
            )
            .await,
    );
    assert!(
        matches!(error, BrowserError::ScriptTimeout { seconds: 30 }),
        "{error}"
    );
    assert!(started.elapsed() >= Duration::from_secs(30));
}

#[tokio::test]
async fn source_reads_the_document_in_the_main_world() {
    let harness = PageHarness::new().await;
    install(&harness, |command| {
        declaration(command)
            .contains("documentElement||{}")
            .then(|| by_value(json!("<html><body></body></html>")))
    });
    let source = harness.page.main_frame().source().await.unwrap();
    assert_eq!(source, "<html><body></body></html>");
    let call = calls_where(&harness, |command| {
        declaration(command).contains("documentElement||{}")
    })
    .remove(0);
    assert_eq!(call.params["uniqueContextId"], "S1-main");
}

fn capture_reply(command: &SentCommand) -> Option<Value> {
    (command.method == "Page.captureScreenshot")
        .then(|| json!({"data": base64::engine::general_purpose::STANDARD.encode(b"png-bytes")}))
}

fn viewport_reply(command: &SentCommand, view: &Value) -> Option<Value> {
    declaration(command)
        .contains("pageXOffset")
        .then(|| by_value(view.clone()))
}

#[tokio::test]
async fn main_frame_screenshots_clip_like_chromedriver() {
    let harness = PageHarness::new().await;
    install(&harness, |command| {
        if let Some(reply) = capture_reply(command).or_else(|| {
            viewport_reply(
                command,
                &json!({"x": 0, "y": 100, "width": 1185, "height": 600}),
            )
        }) {
            return Some(reply);
        }
        if declaration(command).contains("getElementRegion(this)") {
            return Some(by_value(
                json!({"left": 0.4, "top": 0, "width": 87.9, "height": 47}),
            ));
        }
        match atom(command) {
            Some("getLocationInView") => Some(by_value(json!({"x": 150.5, "y": 20.25}))),
            Some("getSize") => Some(by_value(json!({"width": 87, "height": 47}))),
            _ => None,
        }
    });
    let element = mint(&harness.page.main_frame(), "L1", &[8]).await.remove(0);
    assert_eq!(element.screenshot_png().await.unwrap(), b"png-bytes");
    let in_view = calls_where(&harness, |command| {
        atom(command) == Some("getLocationInView")
    })
    .remove(0);
    assert_eq!(
        in_view.params["arguments"],
        json!([{"value": false}, {"value": {"left": 0, "top": 0, "width": 87, "height": 47}}])
    );
    let capture = calls_where(&harness, |command| {
        command.method == "Page.captureScreenshot"
    })
    .remove(0);
    assert_eq!(capture.session.as_deref(), Some("S1"));
    assert_eq!(
        capture.params,
        json!({"format": "png", "clip": {"x": 150.5, "y": 120.25, "width": 87.0, "height": 47.0, "scale": 1}})
    );
}

#[tokio::test]
async fn child_frame_screenshots_add_the_owner_content_origin() {
    let harness = PageHarness::new().await;
    install(&harness, |command| {
        if let Some(reply) = capture_reply(command).or_else(|| {
            viewport_reply(
                command,
                &json!({"x": 0, "y": 10, "width": 800, "height": 600}),
            )
        }) {
            return Some(reply);
        }
        match command.method.as_str() {
            "DOM.scrollIntoViewIfNeeded" => Some(json!({})),
            "DOM.getContentQuads" => {
                Some(json!({"quads": [[20, 50, 108.5, 50, 108.5, 70, 20, 70]]}))
            }
            "DOM.getFrameOwner" => Some(json!({"backendNodeId": 5, "nodeId": 0})),
            "DOM.getBoxModel" => {
                Some(json!({"model": {"content": [51, 297.5, 351, 297.5, 351, 447.5, 51, 447.5]}}))
            }
            _ => None,
        }
    });
    harness.attach_child("S2", "F2", "T1", "http://127.0.0.1/child", "L2");
    let child = harness.page.frame(&"F2".into()).unwrap();
    let element = mint(&child, "L2", &[40]).await.remove(0);
    assert_eq!(element.screenshot_png().await.unwrap(), b"png-bytes");
    let quads = calls_where(&harness, |command| command.method == "DOM.getContentQuads").remove(0);
    assert_eq!(quads.session.as_deref(), Some("S2"));
    let owner = calls_where(&harness, |command| command.method == "DOM.getFrameOwner").remove(0);
    assert_eq!(owner.session.as_deref(), Some("S1"));
    assert_eq!(owner.params, json!({"frameId": "F2"}));
    let capture = calls_where(&harness, |command| {
        command.method == "Page.captureScreenshot"
    })
    .remove(0);
    assert_eq!(
        capture.params["clip"],
        json!({"x": 71.0, "y": 357.5, "width": 88.5, "height": 20.0, "scale": 1})
    );
}

#[tokio::test]
async fn scrolling_and_focus_run_in_the_main_world() {
    let harness = PageHarness::new().await;
    install(&harness, |command| {
        let action = declaration(command);
        (action.contains("scrollIntoView") || action.contains("this.focus()"))
            .then(|| json!({"result": {"type": "undefined"}}))
    });
    let element = mint(&harness.page.main_frame(), "L1", &[8]).await.remove(0);
    element.scroll_into_view().await.unwrap();
    element.focus().await.unwrap();
    let scroll = calls_where(&harness, |command| {
        declaration(command).contains("scrollIntoView")
    })
    .remove(0);
    assert!(declaration(&scroll).contains("block:\"center\",inline:\"center\""));
    let contexts: Vec<i64> = calls_where(&harness, |command| command.method == "DOM.resolveNode")
        .iter()
        .filter_map(|command| command.params["executionContextId"].as_i64())
        .collect();
    assert_eq!(contexts, vec![1, 1]);
}
