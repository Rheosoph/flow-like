use flow_like::flow::{
    board::{Board, Layer, LayerType, cleanup::sync_node_schema::sync_board_node_schemas},
    node::Node,
    pin::{Pin, PinType, ValueType},
    variable::VariableType,
};
use flow_like::state::FlowNodeRegistryInner;
use flow_like_storage::Path;
use flow_like_types::{FromProto, ToProto, Value, json::json};
use std::{collections::BTreeSet, sync::Arc};

struct Case {
    node: &'static str,
    old: &'static str,
    native: &'static str,
    output: bool,
    array: bool,
    mode: &'static str,
}

const CASES: &[Case] = &[
    Case {
        node: "geo_reverse_geocode",
        old: "coordinate",
        native: "geometry",
        output: false,
        array: false,
        mode: "coordinate_to_point",
    },
    Case {
        node: "geo_get_map_image",
        old: "coordinate",
        native: "geometry",
        output: false,
        array: false,
        mode: "coordinate_to_point",
    },
    Case {
        node: "h3_latlng_to_cell",
        old: "coordinate",
        native: "geometry",
        output: false,
        array: false,
        mode: "coordinate_to_point",
    },
    Case {
        node: "h3_cell_to_latlng",
        old: "coordinate",
        native: "geometry_out",
        output: true,
        array: false,
        mode: "point_to_coordinate",
    },
    Case {
        node: "h3_cell_to_boundary",
        old: "boundary",
        native: "geometry_out",
        output: true,
        array: false,
        mode: "h3_boundary",
    },
    Case {
        node: "h3_cells_to_multi_polygon",
        old: "polygons",
        native: "geometry_out",
        output: true,
        array: false,
        mode: "h3_polygons",
    },
    Case {
        node: "geo_plan_route",
        old: "start",
        native: "start_geometry",
        output: false,
        array: false,
        mode: "coordinate_to_point",
    },
    Case {
        node: "geo_plan_route",
        old: "end",
        native: "end_geometry",
        output: false,
        array: false,
        mode: "coordinate_to_point",
    },
    Case {
        node: "geo_plan_route",
        old: "waypoints",
        native: "waypoint_geometries",
        output: false,
        array: false,
        mode: "waypoints_to_points",
    },
    Case {
        node: "geo_plan_route",
        old: "geometry",
        native: "geometry_out",
        output: true,
        array: false,
        mode: "geometry_to_route",
    },
    Case {
        node: "geo_osrm_nearest",
        old: "coordinate",
        native: "geometry",
        output: false,
        array: false,
        mode: "coordinate_to_point",
    },
    Case {
        node: "geo_osrm_table",
        old: "coordinates",
        native: "geometries",
        output: false,
        array: true,
        mode: "coordinates_to_points",
    },
    Case {
        node: "geo_osrm_trip",
        old: "coordinates",
        native: "geometries",
        output: false,
        array: true,
        mode: "coordinates_to_points",
    },
    Case {
        node: "geo_osrm_trip",
        old: "geometry",
        native: "geometry_out",
        output: true,
        array: true,
        mode: "geometry_to_trip_route",
    },
    Case {
        node: "geo_osrm_match_trace",
        old: "coordinates",
        native: "geometries",
        output: false,
        array: true,
        mode: "coordinates_to_points",
    },
    Case {
        node: "geo_get_current_location",
        old: "coordinate",
        native: "geometry",
        output: true,
        array: false,
        mode: "point_to_coordinate",
    },
];

fn registry() -> FlowNodeRegistryInner {
    FlowNodeRegistryInner::prepare(&Arc::new(crate::get_catalog()))
}

fn legacy_node(registry: &FlowNodeRegistryInner, case: &Case, version: Option<u32>) -> Node {
    let mut node = registry.get_node(case.node).unwrap();
    node.id = format!("placed-{}-{}", case.node, case.old);
    node.version = version;
    if version.is_none() {
        node.pins
            .retain(|_, pin| pin.data_type != VariableType::Geometry);
    } else {
        for pin in node.pins.values_mut() {
            if pin.data_type == VariableType::Geometry {
                pin.default_value = None;
            }
        }
    }
    let pin = if case.output {
        node.add_output_pin(case.old, case.old, "", VariableType::Struct)
    } else {
        node.add_input_pin(case.old, case.old, "", VariableType::Struct)
    };
    pin.value_type = if case.array {
        ValueType::Array
    } else {
        ValueType::Normal
    };
    pin.set_schema::<crate::geo::GeoCoordinate>();
    node
}

fn coordinate() -> Value {
    json!({"latitude": 52.52, "longitude": 13.405})
}
fn point() -> Value {
    json!({"type": "Point", "coordinates": [13.405, 52.52]})
}
fn board() -> Board {
    Board::new_detached(Some("migration".into()), Path::default())
}
fn adapters(board: &Board) -> Vec<&Node> {
    board
        .nodes
        .values()
        .chain(board.layers.values().flat_map(|layer| layer.nodes.values()))
        .filter(|node| node.name == "geometry_legacy_adapter")
        .collect()
}
fn pin_mut<'a>(board: &'a mut Board, id: &str) -> &'a mut Pin {
    for node in board.nodes.values_mut() {
        if let Some(pin) = node.pins.get_mut(id) {
            return pin;
        }
    }
    for layer in board.layers.values_mut() {
        if let Some(pin) = layer.pins.get_mut(id) {
            return pin;
        }
        for node in layer.nodes.values_mut() {
            if let Some(pin) = node.pins.get_mut(id) {
                return pin;
            }
        }
    }
    panic!("pin missing: {id}")
}
fn connect(board: &mut Board, source: &str, target: &str) {
    pin_mut(board, source).connected_to.insert(target.into());
    pin_mut(board, target).depends_on.insert(source.into());
}
fn all_pin_ids(board: &Board) -> BTreeSet<String> {
    board
        .nodes
        .values()
        .chain(board.layers.values().flat_map(|layer| layer.nodes.values()))
        .flat_map(|node| node.pins.keys().cloned())
        .collect()
}

#[test]
fn spatial_node_interfaces_expose_geometry_without_retired_struct_pins() {
    let registry = registry();
    for case in CASES {
        let node = registry.get_node(case.node).unwrap();
        assert!(node.version.unwrap_or(0) >= 2, "{}", case.node);
        assert!(
            node.get_pin_by_name(case.old).is_none(),
            "{}.{} remains",
            case.node,
            case.old
        );
        assert_eq!(
            node.get_pin_by_name(case.native).unwrap().data_type,
            VariableType::Geometry
        );
    }
}

#[tokio::test]
async fn legacy_coordinate_literals_migrate_in_boards_and_function_layers() {
    let registry = registry();
    for case in CASES.iter().filter(|case| !case.output) {
        for version in [None, Some(1)] {
            for scoped in [false, true] {
                let mut board = board();
                let mut node = legacy_node(&registry, case, version);
                let node_id = node.id.clone();
                let array = case.mode != "coordinate_to_point";
                node.get_pin_mut_by_name(case.old)
                    .unwrap()
                    .set_default_value(Some(if array {
                        json!([
                            coordinate(),
                            {"latitude": 48.8566, "longitude": 2.3522}
                        ])
                    } else {
                        coordinate()
                    }));
                if scoped {
                    let mut layer =
                        Layer::new("function".into(), "Function".into(), LayerType::Function);
                    node.layer = Some(layer.id.clone());
                    layer.nodes.insert(node.id.clone(), node);
                    board.layers.insert(layer.id.clone(), layer);
                } else {
                    board.nodes.insert(node.id.clone(), node);
                }
                sync_board_node_schemas(&mut board, &registry).await;
                let node = if scoped {
                    &board.layers["function"].nodes[&node_id]
                } else {
                    &board.nodes[&node_id]
                };
                assert!(node.error.is_none(), "{}: {:?}", case.node, node.error);
                assert!(node.get_pin_by_name(case.old).is_none());
                let pin = node.get_pin_by_name(case.native).unwrap();
                assert_eq!(
                    flow_like_types::json::from_slice::<Value>(pin.default_value.as_ref().unwrap())
                        .unwrap(),
                    if array {
                        json!([
                            point(),
                            {"type": "Point", "coordinates": [2.3522, 48.8566]}
                        ])
                    } else {
                        point()
                    }
                );
                assert!(adapters(&board).is_empty());
                let ids = all_pin_ids(&board);
                let mut reloaded = Board::from_proto(board.to_proto());
                sync_board_node_schemas(&mut reloaded, &registry).await;
                assert_eq!(all_pin_ids(&reloaded), ids);
                reloaded.validate_geometry_contracts().unwrap();
            }
        }
    }
}

#[tokio::test]
async fn external_struct_sources_get_adapters_and_supplied_native_inputs_win() {
    let registry = registry();
    let case = &CASES[0];
    for native_supplied in [false, true] {
        for asymmetric in [false, true] {
            let mut board = board();
            let node = legacy_node(&registry, case, Some(1));
            let node_id = node.id.clone();
            let old_id = node.get_pin_by_name(case.old).unwrap().id.clone();
            let native_id = node.get_pin_by_name(case.native).unwrap().id.clone();
            board.nodes.insert(node.id.clone(), node);
            let mut source = Node::new("legacy_source", "Source", "", "Test");
            let source_id = source
                .add_output_pin("coordinate", "Coordinate", "", VariableType::Struct)
                .id
                .clone();
            let native_source = source
                .add_output_pin("geometry", "Geometry", "", VariableType::Geometry)
                .id
                .clone();
            board.nodes.insert(source.id.clone(), source);
            connect(&mut board, &source_id, &old_id);
            if native_supplied {
                connect(&mut board, &native_source, &native_id);
                if asymmetric {
                    pin_mut(&mut board, &native_id).depends_on.clear();
                }
            }
            sync_board_node_schemas(&mut board, &registry).await;
            assert!(board.nodes[&node_id].error.is_none());
            assert!(board.nodes[&node_id].get_pin_by_name(case.old).is_none());
            if native_supplied {
                assert!(adapters(&board).is_empty());
                assert!(
                    !board
                        .get_pin_by_id(&source_id)
                        .unwrap()
                        .connected_to
                        .contains(&old_id)
                );
                assert!(
                    board
                        .get_pin_by_id(&native_id)
                        .unwrap()
                        .depends_on
                        .contains(&native_source)
                );
            } else {
                let adapters = adapters(&board);
                assert_eq!(adapters.len(), 1);
                let value = adapters[0].get_pin_by_name("value").unwrap();
                assert_eq!(value.id, old_id);
                assert!(value.depends_on.contains(&source_id));
                let output = adapters[0].get_pin_by_name("converted").unwrap();
                assert!(output.connected_to.contains(&native_id));
                assert!(
                    board
                        .get_pin_by_id(&native_id)
                        .unwrap()
                        .depends_on
                        .contains(&output.id)
                );
            }
            board.cleanup();
            if native_supplied {
                assert!(
                    board
                        .get_pin_by_id(&native_id)
                        .unwrap()
                        .depends_on
                        .contains(&native_source)
                );
            }
        }
    }
}

#[tokio::test]
async fn supplied_native_literals_win_over_legacy_wires_and_invalid_defaults() {
    let registry = registry();
    let mut board = board();
    let mut node = legacy_node(&registry, &CASES[0], Some(1));
    let old_id = node.get_pin_by_name("coordinate").unwrap().id.clone();
    node.get_pin_mut_by_name("coordinate")
        .unwrap()
        .default_value = Some(b"{broken".to_vec());
    node.get_pin_mut_by_name("geometry")
        .unwrap()
        .set_default_value(Some(point()));
    let native_id = node.get_pin_by_name("geometry").unwrap().id.clone();
    board.nodes.insert(node.id.clone(), node);
    let mut source = Node::new("legacy_source", "Source", "", "Test");
    let source_id = source
        .add_output_pin("coordinate", "Coordinate", "", VariableType::Struct)
        .id
        .clone();
    board.nodes.insert(source.id.clone(), source);
    connect(&mut board, &source_id, &old_id);

    sync_board_node_schemas(&mut board, &registry).await;
    assert!(adapters(&board).is_empty());
    assert!(board.get_pin_by_id(&old_id).is_none());
    assert!(
        board
            .get_pin_by_id(&source_id)
            .unwrap()
            .connected_to
            .is_empty()
    );
    let native = board.get_pin_by_id(&native_id).unwrap();
    assert!(native.depends_on.is_empty());
    assert_eq!(
        flow_like_types::json::from_slice::<Value>(native.default_value.as_ref().unwrap()).unwrap(),
        point()
    );
}

#[tokio::test]
async fn retired_outputs_preserve_both_legacy_and_native_fanout() {
    let registry = registry();
    for case in CASES.iter().filter(|case| case.output) {
        let mut board = board();
        let mut node = legacy_node(&registry, case, Some(1));
        let node_id = node.id.clone();
        let old = node.get_pin_by_name(case.old).unwrap().clone();
        let native = node.get_pin_by_name(case.native).unwrap().clone();
        let source_name = match case.mode {
            "h3_boundary" => Some("cell"),
            "h3_polygons" => Some("cells"),
            _ => None,
        };
        if let Some(name) = source_name {
            node.get_pin_mut_by_name(name)
                .unwrap()
                .set_default_value(Some(if name == "cell" {
                    json!("8928308280fffff")
                } else {
                    json!(["8928308280fffff"])
                }));
        }
        board.nodes.insert(node.id.clone(), node);
        let h3_source = source_name.map(|name| {
            let original = board.nodes[&node_id].get_pin_by_name(name).unwrap();
            let original_id = original.id.clone();
            let mut source = Node::new("h3_source", "H3 Source", "", "Test");
            let output = source.add_output_pin("cells", "Cells", "", VariableType::String);
            output.value_type = original.value_type.clone();
            // The producer half alone must also reach the adapter's copied source.
            output.connected_to.insert(original_id);
            let output_id = output.id.clone();
            board.nodes.insert(source.id.clone(), source);
            output_id
        });
        let mut sink = Node::new("legacy_sink", "Sink", "", "Test");
        let legacy_target = sink
            .add_input_pin("legacy", "Legacy", "", VariableType::Struct)
            .set_value_type(old.value_type.clone())
            .id
            .clone();
        let native_target = sink
            .add_input_pin("native", "Native", "", VariableType::Geometry)
            .id
            .clone();
        board.nodes.insert(sink.id.clone(), sink);
        connect(&mut board, &old.id, &legacy_target);
        // A stale board may retain only the consumer half of a native connection.
        pin_mut(&mut board, &native_target)
            .depends_on
            .insert(native.id.clone());
        sync_board_node_schemas(&mut board, &registry).await;
        assert!(
            board.nodes[&node_id].error.is_none(),
            "{}: {:?}",
            case.node,
            board.nodes[&node_id].error
        );
        assert!(board.nodes[&node_id].get_pin_by_name(case.old).is_none());
        let migrated = adapters(&board);
        assert_eq!(migrated.len(), 1, "{}", case.node);
        let adapter = migrated[0];
        assert_eq!(adapter.get_pin_by_name("converted").unwrap().id, old.id);
        assert_eq!(
            adapter.get_pin_by_name("converted").unwrap().value_type,
            old.value_type
        );
        assert!(
            adapter
                .get_pin_by_name("converted")
                .unwrap()
                .connected_to
                .contains(&legacy_target)
        );
        assert!(
            adapter
                .get_pin_by_name("value")
                .unwrap()
                .depends_on
                .contains(&native.id)
        );
        let mode: Value = flow_like_types::json::from_slice(
            adapter
                .get_pin_by_name("mode")
                .unwrap()
                .default_value
                .as_ref()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(mode, json!(case.mode));
        if let Some(name) = source_name {
            let copied = adapter.get_pin_by_name("source").unwrap();
            assert_eq!(
                copied.default_value,
                board.nodes[&node_id]
                    .get_pin_by_name(name)
                    .unwrap()
                    .default_value
            );
            assert!(!copied.is_optional());
            let source_id = h3_source.as_ref().unwrap();
            assert!(copied.depends_on.contains(source_id));
            assert!(
                board
                    .get_pin_by_id(source_id)
                    .unwrap()
                    .connected_to
                    .contains(&copied.id)
            );
        }
        let input_indices: BTreeSet<_> = adapter
            .pins
            .values()
            .filter(|pin| pin.pin_type == PinType::Input)
            .map(|pin| pin.index)
            .collect();
        assert_eq!(input_indices.len(), 3);
        let ids = all_pin_ids(&board);
        sync_board_node_schemas(&mut board, &registry).await;
        assert_eq!(all_pin_ids(&board), ids);
        board.cleanup();
        assert!(
            board
                .get_pin_by_id(&native.id)
                .unwrap()
                .connected_to
                .contains(&native_target)
        );
        assert!(
            board
                .get_pin_by_id(&legacy_target)
                .unwrap()
                .depends_on
                .contains(&old.id)
        );
    }
}

#[tokio::test]
async fn wired_adapters_stay_inside_function_layers_after_save_and_reload() {
    let registry = registry();
    let mut board = board();
    let mut layer = Layer::new("function".into(), "Function".into(), LayerType::Function);
    let mut node = legacy_node(&registry, &CASES[0], None);
    node.layer = Some(layer.id.clone());
    let node_id = node.id.clone();
    let old_id = node.get_pin_by_name("coordinate").unwrap().id.clone();
    let mut source = Node::new("legacy_source", "Source", "", "Test");
    source.layer = Some(layer.id.clone());
    let output = source.add_output_pin("coordinate", "Coordinate", "", VariableType::Struct);
    let source_id = output.id.clone();
    output.connected_to.insert(old_id.clone());
    node.get_pin_mut_by_name("coordinate")
        .unwrap()
        .depends_on
        .insert(source_id.clone());
    layer.nodes.insert(source.id.clone(), source);
    layer.nodes.insert(node.id.clone(), node);
    board.layers.insert(layer.id.clone(), layer);

    sync_board_node_schemas(&mut board, &registry).await;
    assert!(board.nodes.is_empty());
    assert_eq!(board.layers["function"].nodes.len(), 3);
    let adapter = adapters(&board)[0];
    assert_eq!(adapter.layer.as_deref(), Some("function"));
    let adapter_id = adapter.id.clone();
    let converted_id = adapter.get_pin_by_name("converted").unwrap().id.clone();
    let native_id = board.layers["function"].nodes[&node_id]
        .get_pin_by_name("geometry")
        .unwrap()
        .id
        .clone();
    let ids = all_pin_ids(&board);

    let mut board = Board::from_proto(board.to_proto());
    sync_board_node_schemas(&mut board, &registry).await;
    board.cleanup();
    assert_eq!(all_pin_ids(&board), ids);
    assert!(board.layers["function"].nodes.contains_key(&adapter_id));
    for (source, target) in [(&source_id, &old_id), (&converted_id, &native_id)] {
        assert!(
            board
                .get_pin_by_id(source)
                .unwrap()
                .connected_to
                .contains(target)
        );
        assert!(
            board
                .get_pin_by_id(target)
                .unwrap()
                .depends_on
                .contains(source)
        );
    }
    board.validate_geometry_contracts().unwrap();
}

#[tokio::test]
async fn native_point_chains_migrate_directly_and_keep_other_legacy_consumers() {
    let registry = registry();
    for version in [None, Some(1)] {
        for legacy_fanout in [false, true] {
            let mut board = board();
            let source = legacy_node(&registry, &CASES[3], version);
            let source_id = source.id.clone();
            let old_output = source.get_pin_by_name("coordinate").unwrap().id.clone();
            let target = legacy_node(&registry, &CASES[0], version);
            let target_id = target.id.clone();
            let old_input = target.get_pin_by_name("coordinate").unwrap().id.clone();
            board.nodes.insert(source.id.clone(), source);
            board.nodes.insert(target.id.clone(), target);
            connect(&mut board, &old_output, &old_input);
            if legacy_fanout {
                let mut sink = Node::new("legacy_sink", "Sink", "", "Test");
                let input = sink
                    .add_input_pin("coordinate", "Coordinate", "", VariableType::Struct)
                    .id
                    .clone();
                board.nodes.insert(sink.id.clone(), sink);
                connect(&mut board, &old_output, &input);
            }
            sync_board_node_schemas(&mut board, &registry).await;
            assert!(board.nodes[&source_id].error.is_none());
            assert!(board.nodes[&target_id].error.is_none());
            assert_eq!(adapters(&board).len(), usize::from(legacy_fanout));
            let output = board.nodes[&source_id]
                .get_pin_by_name("geometry_out")
                .unwrap();
            let input = board.nodes[&target_id].get_pin_by_name("geometry").unwrap();
            assert!(output.connected_to.contains(&input.id));
            assert!(input.depends_on.contains(&output.id));
        }
    }
}

#[tokio::test]
async fn invalid_literals_are_retained_on_adapters_and_unset_values_need_none() {
    let registry = registry();
    for bytes in [
        None,
        Some(Vec::new()),
        Some(b"null".to_vec()),
        Some(b"{broken".to_vec()),
        Some(b"{}".to_vec()),
    ] {
        let mut board = board();
        let mut node = legacy_node(&registry, &CASES[0], None);
        let old_id = node.get_pin_by_name("coordinate").unwrap().id.clone();
        node.get_pin_mut_by_name("coordinate")
            .unwrap()
            .default_value = bytes.clone();
        let node_id = node.id.clone();
        board.nodes.insert(node.id.clone(), node);
        sync_board_node_schemas(&mut board, &registry).await;
        assert!(board.nodes[&node_id].error.is_none());
        assert!(
            board.nodes[&node_id]
                .get_pin_by_name("coordinate")
                .is_none()
        );
        let invalid = bytes
            .as_ref()
            .is_some_and(|value| !value.is_empty() && value != b"null");
        assert_eq!(adapters(&board).len(), usize::from(invalid));
        if invalid {
            assert_eq!(board.get_pin_by_id(&old_id).unwrap().default_value, bytes);
        }
    }
}

#[tokio::test]
async fn missing_adapters_and_id_collisions_leave_the_original_graph_intact() {
    for collision in [false, true] {
        let mut registry = registry();
        let mut board = board();
        let mut node = legacy_node(&registry, &CASES[0], None);
        let old_id = node.get_pin_by_name("coordinate").unwrap().id.clone();
        if collision {
            let mut unrelated = node
                .add_input_pin("unrelated", "Unrelated", "", VariableType::String)
                .clone();
            node.pins.remove(&unrelated.id);
            unrelated.id = format!("{old_id}-geometry");
            node.pins.insert(unrelated.id.clone(), unrelated);
        } else {
            registry.registry.remove("geometry_legacy_adapter");
            node.get_pin_mut_by_name("coordinate")
                .unwrap()
                .default_value = Some(b"{broken".to_vec());
        }
        let node_id = node.id.clone();
        let before = node.pins.clone();
        board.nodes.insert(node.id.clone(), node);
        sync_board_node_schemas(&mut board, &registry).await;
        assert_eq!(board.nodes.len(), 1);
        assert_eq!(board.nodes[&node_id].version, None);
        assert!(
            board.nodes[&node_id]
                .error
                .as_ref()
                .unwrap()
                .contains("migration failed")
        );
        for (id, pin) in before {
            let after = &board.nodes[&node_id].pins[&id];
            assert_eq!(after.name, pin.name);
            assert_eq!(after.default_value, pin.default_value);
            assert_eq!(after.depends_on, pin.depends_on);
        }
    }
}
