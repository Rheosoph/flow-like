extern crate flow_like_runtime as flow_like;

use flow_like::{
    flow::{
        board::{Board, Layer, LayerType, cleanup::sync_node_schema::sync_board_node_schemas},
        node::Node,
        pin::{Pin, PinOptions, ValueType},
        variable::VariableType,
    },
    state::FlowNodeRegistryInner,
};
use flow_like_catalog_automation::get_catalog;
use serde_json::{Value, json};

fn registry() -> FlowNodeRegistryInner {
    FlowNodeRegistryInner::from_registry(
        get_catalog()
            .into_iter()
            .map(|logic| {
                let node = logic.get_node();
                (node.name.clone(), (node, logic))
            })
            .collect(),
    )
}

fn board() -> Board {
    Board::new_detached(Some("migration-test".into()), Default::default())
}

fn cases() -> Vec<(&'static str, &'static str, u32, Value)> {
    vec![
        (
            "browser_upload_multiple_files",
            "file_paths",
            1,
            json!(["/tmp/Zoë.txt", "/tmp/文.csv"]),
        ),
        (
            "llm_observe_screen",
            "elements",
            4,
            json!([{"element_type":"button","description":"Save","approximate_location":"top right","is_interactive":true,"current_state":null}]),
        ),
        (
            "llm_plan_actions",
            "actions",
            4,
            json!([{"action_type":"click","target":"#save","parameters":{},"reasoning":"Save changes","expected_result":"Saved"}]),
        ),
        (
            "llm_resolve_element",
            "candidates",
            4,
            json!([{"index":0,"x":10,"y":20,"description":"Save"}]),
        ),
        (
            "llm_rank_candidates",
            "candidates",
            4,
            json!([{"id":"save","description":"Save","x":10,"y":20,"selector":"#save","additional_info":null}]),
        ),
        (
            "llm_rank_candidates",
            "ranked",
            4,
            json!([{"id":"save","rank":1,"score":0.95,"reasoning":"Matches intent","is_recommended":true}]),
        ),
    ]
}

fn legacy_node(
    registry: &FlowNodeRegistryInner,
    name: &str,
    pin_name: &str,
    value_type: ValueType,
    version: Option<u32>,
    bytes: Vec<u8>,
) -> Node {
    let mut node = registry.get_node(name).unwrap();
    node.version = version;
    let pin = node.get_pin_mut_by_name(pin_name).unwrap();
    pin.data_type = VariableType::Generic;
    pin.value_type = value_type;
    pin.schema = None;
    pin.default_value = Some(bytes);
    node
}

fn connect(source: &mut Node, source_pin: &str, target: &mut Node, target_pin: &str) {
    let output_id = source.get_pin_by_name(source_pin).unwrap().id.clone();
    let input_id = target.get_pin_by_name(target_pin).unwrap().id.clone();
    source
        .get_pin_mut_by_name(source_pin)
        .unwrap()
        .connected_to
        .insert(input_id);
    target
        .get_pin_mut_by_name(target_pin)
        .unwrap()
        .depends_on
        .insert(output_id);
}

fn assert_edge(source: &Pin, target: &Pin, expected: bool) {
    assert_eq!(source.connected_to.contains(&target.id), expected);
    assert_eq!(target.depends_on.contains(&source.id), expected);
}

#[tokio::test]
async fn six_legacy_array_contracts_preserve_valid_literal_bytes_and_pin_identity() {
    let registry = registry();
    for (name, pin_name, version, value) in cases() {
        let catalog = registry.get_node(name).unwrap();
        assert_eq!(catalog.version, Some(version), "{name}");
        let catalog_pin = catalog.get_pin_by_name(pin_name).unwrap();
        for value_type in [ValueType::Normal, ValueType::Array] {
            for placed_version in [None, Some(version - 1)] {
                let bytes = serde_json::to_vec_pretty(&value).unwrap();
                let node = legacy_node(
                    &registry,
                    name,
                    pin_name,
                    value_type.clone(),
                    placed_version,
                    bytes.clone(),
                );
                let node_id = node.id.clone();
                let pin_id = node.get_pin_by_name(pin_name).unwrap().id.clone();
                let mut board = board();
                board.nodes.insert(node_id.clone(), node);
                sync_board_node_schemas(&mut board, &registry).await;
                let migrated = &board.nodes[&node_id];
                let pin = migrated.get_pin_by_name(pin_name).unwrap();
                assert_eq!(migrated.version, Some(version), "{name}:{pin_name}");
                assert_eq!(pin.id, pin_id, "{name}:{pin_name}");
                assert_eq!(
                    pin.default_value.as_ref(),
                    Some(&bytes),
                    "{name}:{pin_name}"
                );
                assert_eq!(pin.data_type, catalog_pin.data_type, "{name}:{pin_name}");
                assert_eq!(pin.value_type, ValueType::Array, "{name}:{pin_name}");
                assert_eq!(pin.schema, catalog_pin.schema, "{name}:{pin_name}");
            }
        }
    }
}

#[tokio::test]
async fn invalid_items_and_scalar_defaults_reset_to_the_catalog_default() {
    let registry = registry();
    for (name, pin_name, version, _) in cases() {
        let catalog = registry.get_node(name).unwrap();
        for invalid in [json!([false]), json!([{}]), json!("scalar"), json!({})] {
            let node = legacy_node(
                &registry,
                name,
                pin_name,
                ValueType::Normal,
                Some(version - 1),
                serde_json::to_vec(&invalid).unwrap(),
            );
            let node_id = node.id.clone();
            let mut board = board();
            board.nodes.insert(node_id.clone(), node);
            sync_board_node_schemas(&mut board, &registry).await;
            let pin = board.nodes[&node_id].get_pin_by_name(pin_name).unwrap();
            assert_eq!(
                pin.default_value,
                catalog.get_pin_by_name(pin_name).unwrap().default_value,
                "{name}:{pin_name} accepted {invalid}"
            );
            assert_eq!(pin.value_type, ValueType::Array);
        }
    }
}

#[tokio::test]
async fn migrated_inputs_preserve_typed_producers_and_prune_incompatible_or_unknown_sources() {
    let registry = registry();
    let catalog = registry.get_node("llm_rank_candidates").unwrap();
    let target_contract = catalog.get_pin_by_name("candidates").unwrap();
    for (data_type, value_type, schema, keep) in [
        (
            VariableType::Struct,
            ValueType::Array,
            target_contract.schema.clone(),
            true,
        ),
        (VariableType::String, ValueType::Array, None, false),
        (
            VariableType::Struct,
            ValueType::Normal,
            target_contract.schema.clone(),
            false,
        ),
        (VariableType::Generic, ValueType::Array, None, false),
        (
            VariableType::Struct,
            ValueType::Array,
            Some(json!({"type":"object","properties":{"wrong":{"type":"boolean"}},"required":["wrong"]}).to_string()),
            false,
        ),
    ] {
        let mut source = Node::new("test_producer", "Producer", "", "Test");
        let output = source.add_output_pin("value", "Value", "", data_type);
        output.value_type = value_type;
        output.schema = schema;
        let source_id = source.id.clone();
        let mut target = legacy_node(
            &registry,
            "llm_rank_candidates",
            "candidates",
            ValueType::Normal,
            Some(3),
            b"[]".to_vec(),
        );
        let target_id = target.id.clone();
        connect(&mut source, "value", &mut target, "candidates");
        let mut board = board();
        board.nodes.insert(source_id.clone(), source);
        board.nodes.insert(target_id.clone(), target);
        sync_board_node_schemas(&mut board, &registry).await;
        assert_edge(
            board.nodes[&source_id].get_pin_by_name("value").unwrap(),
            board.nodes[&target_id].get_pin_by_name("candidates").unwrap(),
            keep,
        );
    }
}

#[tokio::test]
async fn migrated_outputs_keep_generic_consumers_except_enforced_container_mismatches() {
    let registry = registry();
    for (value_type, enforce, keep) in [
        (ValueType::Normal, false, true),
        (ValueType::Array, true, true),
        (ValueType::Normal, true, false),
    ] {
        let mut source = legacy_node(
            &registry,
            "llm_plan_actions",
            "actions",
            ValueType::Normal,
            Some(3),
            b"[]".to_vec(),
        );
        let source_id = source.id.clone();
        let mut target = Node::new("test_consumer", "Consumer", "", "Test");
        let input = target.add_input_pin("value", "Value", "", VariableType::Generic);
        input.value_type = value_type;
        input.options = Some(
            PinOptions::new()
                .set_enforce_generic_value_type(enforce)
                .build(),
        );
        let target_id = target.id.clone();
        connect(&mut source, "actions", &mut target, "value");
        let mut board = board();
        board.nodes.insert(source_id.clone(), source);
        board.nodes.insert(target_id.clone(), target);
        sync_board_node_schemas(&mut board, &registry).await;
        assert_edge(
            board.nodes[&source_id].get_pin_by_name("actions").unwrap(),
            board.nodes[&target_id].get_pin_by_name("value").unwrap(),
            keep,
        );
    }
}

#[tokio::test]
async fn nested_layers_migrate_cross_layer_wires_and_second_sync_is_idempotent() {
    let registry = registry();
    let mut target = legacy_node(
        &registry,
        "browser_upload_multiple_files",
        "file_paths",
        ValueType::Normal,
        None,
        b"[ \"/tmp/original.txt\" ]".to_vec(),
    );
    let target_id = target.id.clone();
    let mut source = Node::new("test_paths", "Paths", "", "Test");
    source
        .add_output_pin("paths", "Paths", "", VariableType::String)
        .set_value_type(ValueType::Array);
    let source_id = source.id.clone();
    connect(&mut source, "paths", &mut target, "file_paths");
    let parent = Layer::new("parent".into(), "Parent".into(), LayerType::Module);
    let mut child = Layer::new("child".into(), "Child".into(), LayerType::Module);
    child.parent_id = Some(parent.id.clone());
    child.nodes.insert(target_id.clone(), target);
    let child_id = child.id.clone();
    let mut board = board();
    board.nodes.insert(source_id.clone(), source);
    board.layers.insert(parent.id.clone(), parent);
    board.layers.insert(child.id.clone(), child);
    sync_board_node_schemas(&mut board, &registry).await;
    let target = board.layers[&child_id].nodes[&target_id]
        .get_pin_by_name("file_paths")
        .unwrap();
    assert_eq!(target.data_type, VariableType::String);
    assert_eq!(target.value_type, ValueType::Array);
    assert_eq!(
        target.default_value.as_deref(),
        Some(b"[ \"/tmp/original.txt\" ]".as_slice())
    );
    assert_edge(
        board.nodes[&source_id].get_pin_by_name("paths").unwrap(),
        target,
        true,
    );
    let once = serde_json::to_value(&board).unwrap();
    sync_board_node_schemas(&mut board, &registry).await;
    assert_eq!(serde_json::to_value(&board).unwrap(), once);
}

#[tokio::test]
async fn current_or_future_placed_versions_are_not_special_migrated() {
    let registry = registry();
    for (name, pin_name, version, value) in cases() {
        for placed_version in [version, version + 1] {
            let bytes = serde_json::to_vec_pretty(&value).unwrap();
            let node = legacy_node(
                &registry,
                name,
                pin_name,
                ValueType::Normal,
                Some(placed_version),
                bytes.clone(),
            );
            let node_id = node.id.clone();
            let mut board = board();
            board.nodes.insert(node_id.clone(), node);
            sync_board_node_schemas(&mut board, &registry).await;
            let migrated = &board.nodes[&node_id];
            let pin = migrated.get_pin_by_name(pin_name).unwrap();
            assert_eq!(migrated.version, Some(placed_version));
            assert_eq!(pin.data_type, VariableType::Generic);
            assert_eq!(pin.value_type, ValueType::Normal);
            assert_eq!(pin.default_value.as_ref(), Some(&bytes));
        }
    }
}

#[tokio::test]
async fn unsupported_catalog_versions_unrelated_nodes_and_nonlegacy_types_use_normal_sync() {
    for variant in ["catalog-version", "unrelated-node", "nonlegacy-type"] {
        let mut registry = registry();
        let original = "browser_upload_multiple_files";
        let name = if variant == "unrelated-node" {
            "unrelated_upload"
        } else {
            original
        };
        if name != original {
            let (mut node, logic) = registry.registry[original].clone();
            node.name = name.into();
            registry.registry.insert(name.into(), (node, logic));
        }
        let mut placed = legacy_node(
            &registry,
            name,
            "file_paths",
            ValueType::Normal,
            Some(0),
            b"[\"/tmp/a.txt\"]".to_vec(),
        );
        if variant == "catalog-version" {
            registry.registry.get_mut(name).unwrap().0.version = Some(2);
        } else if variant == "nonlegacy-type" {
            placed.get_pin_mut_by_name("file_paths").unwrap().data_type = VariableType::Boolean;
        }
        let node_id = placed.id.clone();
        let expected = registry.get_node(name).unwrap();
        let mut board = board();
        board.nodes.insert(node_id.clone(), placed);
        sync_board_node_schemas(&mut board, &registry).await;
        let pin = board.nodes[&node_id].get_pin_by_name("file_paths").unwrap();
        assert_eq!(pin.data_type, VariableType::String, "{variant}");
        assert_eq!(pin.value_type, ValueType::Array, "{variant}");
        assert_eq!(
            pin.default_value,
            expected
                .get_pin_by_name("file_paths")
                .unwrap()
                .default_value,
            "{variant}"
        );
    }
}
