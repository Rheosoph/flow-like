mod flowscript_support;

use flow_like::a2ui::{SurfaceComponent, widget::Page};
use flow_like::flow::ast::{RenderOptions, apply_flowscript_to_board, board_to_flowscript};
use flow_like::flow::board::Board;
use flow_like::flow::node::Node;
use flow_like::flow::variable::VariableType;
use flow_like::state::{FlowLikeConfig, FlowLikeState};
use flow_like::utils::http::HTTPClient;
use flow_like_storage::{
    files::store::FlowLikeStore, object_store::memory::InMemory, object_store::path::Path,
};
use flow_like_types::json::json;
use flowscript_support::{CATALOG, all_nodes};
use std::sync::Arc;

async fn widget_board() -> (Board, Arc<FlowLikeState>) {
    let mut config = FlowLikeConfig::new();
    config.register_app_meta_store(FlowLikeStore::Other(Arc::new(InMemory::new())));
    let state = Arc::new(FlowLikeState::new(
        config,
        HTTPClient::new_without_refetch(),
    ));
    state
        .node_registry()
        .write()
        .await
        .push_nodes(CATALOG.logic.clone());
    let mut board = Board::new(
        Some("widget-query-apply".into()),
        Path::from("apps/widget-query-apply"),
        state.clone(),
    );
    let page = Page::new("page", "Chat", "/")
        .with_component(widget_component("chat", "boolean"))
        .with_component(widget_component("counter", "integer"));
    board.save_page(&page, None).await.unwrap();
    (board, state)
}

fn widget_component(id: &str, result_type: &str) -> SurfaceComponent {
    SurfaceComponent::new(
        id,
        json!({
            "type": "microWidgetInstance",
            "packageId": "com.example.chat",
            "packageVersion": "1.0.0",
            "widgetId": id,
            "instanceId": format!("placed-{id}"),
            "contract": {
                "contractVersion": 1,
                "id": id,
                "queries": {
                    "setMessage": {
                        "mutation": true,
                        "argsSchema": {
                            "type": "object",
                            "properties": { "message": { "type": "string" } },
                            "required": ["message"]
                        },
                        "resultSchema": { "type": result_type }
                    }
                }
            }
        }),
    )
}

async fn apply(board: &mut Board, state: &Arc<FlowLikeState>, source: &str) {
    let result =
        apply_flowscript_to_board(board, source, &CATALOG.nodes, state.clone(), None, true)
            .await
            .unwrap_or_else(|error| {
                panic!("widget query applies without rollback: {error}\n{source}")
            });
    assert!(
        result.diagnostics.is_empty(),
        "{:?}\n{source}",
        result.diagnostics
    );
}

fn query(board: &Board) -> &Node {
    let mut queries = all_nodes(board)
        .into_iter()
        .filter(|node| node.name == "a2ui_widget_query");
    let query = queries.next().expect("Query Widget was created");
    assert!(queries.next().is_none(), "the edit must preserve one query");
    query
}

fn assert_message(board: &Board) {
    let query = query(board);
    assert_eq!(query.error, None);
    let message = query
        .get_pin_by_name("dyn_arg_message")
        .expect("the persisted contract supplies the message argument");
    assert_eq!(message.data_type, VariableType::String);
    assert_eq!(
        flow_like_types::json::from_slice::<flow_like_types::Value>(
            message.default_value.as_deref().unwrap()
        )
        .unwrap(),
        json!("Hello")
    );
    assert!(
        !query
            .get_pin_by_name("element_ref")
            .unwrap()
            .depends_on
            .is_empty()
    );
}

async fn assert_repeated_apply(source: &str) {
    let (mut board, state) = widget_board().await;
    apply(&mut board, &state, source).await;
    assert_stable_query(&mut board, &state).await;
}

async fn assert_stable_query(board: &mut Board, state: &Arc<FlowLikeState>) {
    assert_message(board);
    let query_id = query(board).id.clone();
    let message_id = query(board)
        .get_pin_by_name("dyn_arg_message")
        .unwrap()
        .id
        .clone();
    for _ in 0..3 {
        let rendered = board_to_flowscript(
            board,
            &RenderOptions {
                anchors: true,
                ..RenderOptions::default()
            },
        );
        apply(board, state, &rendered).await;
        assert_message(board);
        assert_eq!(query(board).id, query_id);
        assert_eq!(
            query(board).get_pin_by_name("dyn_arg_message").unwrap().id,
            message_id
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn unanchored_widget_query_uses_the_saved_page_contract() {
    assert_repeated_apply(
        r#"detached {
    ui::widgetQuery({ elementRef: ui::getElement({ elementRef: "page/chat" }).element, query: "setMessage", dynArgMessage: "Hello" })
}"#,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn widget_query_accepts_a_const_get_element_selector() {
    assert_repeated_apply(
        r#"const chatElement: string = "page/chat"

detached {
    ui::widgetQuery({ elementRef: ui::getElement({ elementRef: chatElement }).element, query: "setMessage", dynArgMessage: "Hello" })
}"#,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn widget_query_accepts_a_local_const_get_element_selector() {
    assert_repeated_apply(
        r#"detached {
    const chatElement = "page/chat"
    ui::widgetQuery({ elementRef: ui::getElement({ elementRef: chatElement }).element, query: "setMessage", dynArgMessage: "Hello" })
}"#,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_anchored_block_accepts_a_new_unanchored_widget_query() {
    let (mut board, state) = widget_board().await;
    apply(
        &mut board,
        &state,
        r#"detached {
    log::info({ message: "Sending", toast: false })
}"#,
    )
    .await;
    let mut source = board_to_flowscript(
        &board,
        &RenderOptions {
            anchors: true,
            ..RenderOptions::default()
        },
    );
    let block_end = source.rfind('}').expect("rendered detached block");
    source.insert_str(block_end, r#"    ui::widgetQuery({ elementRef: ui::getElement({ elementRef: "page/chat" }).element, query: "setMessage", dynArgMessage: "Hello" })
"#);
    apply(&mut board, &state, &source).await;
    assert_stable_query(&mut board, &state).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn editing_the_const_selector_refreshes_the_query_contract() {
    let (mut board, state) = widget_board().await;
    apply(
        &mut board,
        &state,
        r#"const chatElement: string = "page/chat"

detached {
    ui::widgetQuery({ elementRef: ui::getElement({ elementRef: chatElement }).element, query: "setMessage", dynArgMessage: "Hello" })
}"#,
    )
    .await;
    assert_message(&board);
    let query_id = query(&board).id.clone();
    assert_eq!(
        query(&board).get_pin_by_name("value").unwrap().data_type,
        VariableType::Boolean
    );
    let source = board_to_flowscript(
        &board,
        &RenderOptions {
            anchors: true,
            ..RenderOptions::default()
        },
    )
    .replace("\"page/chat\"", "\"page/counter\"");
    apply(&mut board, &state, &source).await;

    assert_message(&board);
    let query = query(&board);
    assert_eq!(query.id, query_id);
    assert_eq!(
        query.get_pin_by_name("value").unwrap().data_type,
        VariableType::Integer
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_written_selector_variable_cannot_supply_a_static_query_contract() {
    let (mut board, state) = widget_board().await;
    let original = flow_like_types::json::to_value(&board).unwrap();
    let source = r#"const chatElement: string = "page/chat"

detached {
    chatElement = "page/counter"
    ui::widgetQuery({ elementRef: ui::getElement({ elementRef: chatElement }).element, query: "setMessage", dynArgMessage: "Hello" })
}"#;
    let result =
        apply_flowscript_to_board(&mut board, source, &CATALOG.nodes, state, None, true).await;
    match result {
        Ok(result) => assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.contains("dynArgMessage")),
            "the unresolved argument must explain the failure: {:?}",
            result.diagnostics
        ),
        Err(error) => assert!(
            error.to_string().contains("dynArgMessage"),
            "the unresolved argument must explain the failure: {error}"
        ),
    }
    assert_eq!(flow_like_types::json::to_value(&board).unwrap(), original);
}
