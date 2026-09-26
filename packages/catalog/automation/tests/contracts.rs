extern crate flow_like_runtime as flow_like;
use flow_like::flow::{pin::ValueType, variable::VariableType};
use flow_like_catalog_automation::get_catalog;

#[test]
fn collection_contracts_preserve_item_types() {
    let catalog = get_catalog();
    for (name, pin_name, data_type) in [
        (
            "browser_upload_multiple_files",
            "file_paths",
            VariableType::String,
        ),
        ("browser_list_tabs", "tabs", VariableType::Struct),
        ("llm_plan_actions", "actions", VariableType::Struct),
        ("llm_observe_screen", "elements", VariableType::Struct),
        ("llm_resolve_element", "candidates", VariableType::Struct),
        ("llm_rank_candidates", "candidates", VariableType::Struct),
        ("llm_rank_candidates", "ranked", VariableType::Struct),
    ] {
        let node = catalog
            .iter()
            .map(|logic| logic.get_node())
            .find(|node| node.name == name)
            .unwrap();
        let pin = node.pins.values().find(|pin| pin.name == pin_name).unwrap();
        assert_eq!(pin.value_type, ValueType::Array, "{name}:{pin_name}");
        assert_eq!(pin.data_type, data_type, "{name}:{pin_name}");
        if data_type == VariableType::Struct {
            assert!(pin.schema.is_some(), "{name}:{pin_name}");
        }
    }
}

#[test]
fn browser_actions_accept_typed_locators_without_removing_css_pins() {
    let catalog = get_catalog();
    for name in [
        "browser_click",
        "browser_double_click",
        "browser_type_text",
        "browser_wait_for",
        "browser_get_text",
        "browser_screenshot_element",
    ] {
        let node = catalog
            .iter()
            .map(|logic| logic.get_node())
            .find(|node| node.name == name)
            .unwrap();
        let selector = node
            .pins
            .values()
            .find(|pin| pin.name == "selector")
            .unwrap();
        assert_eq!(selector.data_type, VariableType::String);
        let locator = node
            .pins
            .values()
            .find(|pin| pin.name == "locator")
            .unwrap();
        assert_eq!(locator.data_type, VariableType::Struct);
        assert!(locator.is_optional());
        assert!(locator.schema.is_some());
    }
}

#[test]
fn plan_producer_and_executor_share_schema() {
    let nodes: Vec<_> = get_catalog().iter().map(|logic| logic.get_node()).collect();
    let pin = |name: &str| {
        nodes
            .iter()
            .find(|node| node.name == name)
            .unwrap()
            .pins
            .values()
            .find(|pin| pin.name == "plan")
            .unwrap()
    };
    assert_eq!(
        pin("llm_plan_actions").schema,
        pin("browser_execute_plan").schema
    );
    let planner = nodes
        .iter()
        .find(|node| node.name == "llm_plan_actions")
        .unwrap();
    let target = planner
        .pins
        .values()
        .find(|pin| pin.name == "execution_target")
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(target.default_value.as_ref().unwrap())
            .unwrap(),
        "General"
    );
    assert!(target.is_optional());
}

#[test]
fn authentication_passwords_are_sensitive() {
    let node = get_catalog()
        .iter()
        .map(|logic| logic.get_node())
        .find(|node| node.name == "browser_set_basic_auth")
        .unwrap();
    let password = node
        .pins
        .values()
        .find(|pin| pin.name == "password")
        .unwrap();
    assert_eq!(
        password
            .options
            .as_ref()
            .and_then(|options| options.sensitive),
        Some(true)
    );
    let credentials = flow_like_catalog_automation::browser::auth::BasicAuthCredentials {
        username: "user".into(),
        password: "secret-value".into(),
    };
    assert!(!format!("{credentials:?}").contains("secret-value"));
}

#[test]
fn native_and_browser_nodes_request_their_declared_resources() {
    use flow_like_catalog_automation::capabilities::{
        AutomationCapability::*, required_capabilities,
    };
    let nodes: Vec<_> = get_catalog().iter().map(|logic| logic.get_node()).collect();
    for (name, expected) in [
        ("browser_attach", vec![Browser]),
        ("browser_start_driver", vec![Browser, ApplicationLaunch]),
        ("browser_stop_driver", vec![Browser]),
        ("browser_list_tabs", vec![Browser]),
        ("browser_select_tab", vec![Browser]),
        ("browser_enter_frame", vec![Browser]),
        ("browser_leave_frame", vec![Browser]),
        ("browser_handle_dialog", vec![Browser]),
        ("browser_execute_plan", vec![Browser]),
        ("browser_right_click", vec![Browser]),
        ("browser_drag", vec![Browser]),
        ("browser_key_chord", vec![Browser]),
        ("browser_wait_for_url", vec![Browser]),
        ("browser_start_console_observer", vec![Browser]),
        ("computer_launch_app", vec![ApplicationLaunch]),
        ("computer_key_type", vec![InputControl]),
        ("computer_accessibility_action", vec![Accessibility]),
        ("computer_window_operation", vec![WindowManagement]),
        ("computer_wait_for_window", vec![WindowManagement]),
        ("computer_capture_window", vec![ScreenCapture]),
        ("rpa_retry_loop", vec![]),
        ("automation_check_capability", vec![]),
        ("automation_request_capability", vec![]),
    ] {
        assert!(
            nodes.iter().any(|node| node.name == name),
            "Node {name} is not registered"
        );
        assert_eq!(required_capabilities(name), expected, "{name}");
    }
    for node in nodes
        .iter()
        .filter(|node| node.name.starts_with("browser_") && node.name != "browser_start_driver")
    {
        assert_eq!(
            required_capabilities(&node.name),
            vec![Browser],
            "{}",
            node.name
        );
    }
}
