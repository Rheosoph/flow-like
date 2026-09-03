use flow_like::flow::board::Board;
use flow_like::flow::compiled::{
    compile_board, decode_artifact, encode_artifact, format::NONE_IDX, peek_header,
    view::reconstruct_board,
};
use flow_like::flow::node::Node;
use flow_like::flow::pin::{Pin, PinOptions, PinType, ValueType};
use flow_like::flow::variable::{Variable, VariableType};
use flow_like_storage::Path;
use std::collections::BTreeSet;

fn pin(id: &str, name: &str, pin_type: PinType, data_type: VariableType, index: u16) -> Pin {
    Pin {
        id: id.to_string(),
        name: name.to_string(),
        friendly_name: name.to_string(),
        description: format!("{name} description"),
        pin_type,
        data_type,
        schema: None,
        value_type: ValueType::Normal,
        depends_on: BTreeSet::new(),
        connected_to: BTreeSet::new(),
        default_value: None,
        index,
        options: None,
        value: None,
    }
}

fn node_with_pins(id: &str, name: &str, pins: Vec<Pin>) -> Node {
    let mut node = Node::new(name, name, &format!("{name} node"), "Tests");
    node.id = id.to_string();
    node.pins = pins.into_iter().map(|p| (p.id.clone(), p)).collect();
    node
}

fn connect(from: &mut Pin, to: &mut Pin) {
    from.connected_to.insert(to.id.clone());
    to.depends_on.insert(from.id.clone());
}

fn empty_board() -> Board {
    Board::new_detached(
        Some("test-board".to_string()),
        Path::from("apps").child("test"),
    )
}

/// producer.out -> reroute -> reroute -> consumer.in must splice to a direct
/// producer -> consumer edge and drop both reroute nodes.
#[test]
fn reroute_chains_are_spliced() {
    let mut out_pin = pin(
        "p_out",
        "value_out",
        PinType::Output,
        VariableType::String,
        0,
    );
    let mut r1_in = pin(
        "r1_in",
        "route_in",
        PinType::Input,
        VariableType::Generic,
        0,
    );
    let mut r1_out = pin(
        "r1_out",
        "route_out",
        PinType::Output,
        VariableType::Generic,
        1,
    );
    let mut r2_in = pin(
        "r2_in",
        "route_in",
        PinType::Input,
        VariableType::Generic,
        0,
    );
    let mut r2_out = pin(
        "r2_out",
        "route_out",
        PinType::Output,
        VariableType::Generic,
        1,
    );
    let mut in_pin = pin("p_in", "value_in", PinType::Input, VariableType::String, 0);

    connect(&mut out_pin, &mut r1_in);
    connect(&mut r1_out, &mut r2_in);
    connect(&mut r2_out, &mut in_pin);

    let producer = node_with_pins("n_producer", "producer", vec![out_pin]);
    let r1 = node_with_pins("n_r1", "reroute", vec![r1_in, r1_out]);
    let r2 = node_with_pins("n_r2", "reroute", vec![r2_in, r2_out]);
    let consumer = node_with_pins("n_consumer", "consumer", vec![in_pin]);

    let mut board = empty_board();
    for node in [producer, r1, r2, consumer] {
        board.nodes.insert(node.id.clone(), node);
    }

    let compiled = compile_board(&board).expect("compile");

    assert_eq!(compiled.nodes.len(), 2, "reroute nodes must be dropped");
    assert_eq!(compiled.pins.len(), 2, "reroute pins must be dropped");

    let out_idx = compiled.pins.iter().position(|p| p.id == "p_out").unwrap();
    let in_idx = compiled.pins.iter().position(|p| p.id == "p_in").unwrap();
    assert_eq!(compiled.pins[out_idx].connected_to, vec![in_idx as u32]);
    assert_eq!(compiled.pins[in_idx].depends_on, vec![out_idx as u32]);
}

#[test]
fn artifact_roundtrip_preserves_compiled_board() {
    let mut out_pin = pin(
        "p_out",
        "value_out",
        PinType::Output,
        VariableType::String,
        0,
    );
    let mut in_pin = pin("p_in", "value_in", PinType::Input, VariableType::String, 0);
    in_pin.default_value = Some(b"\"fallback\"".to_vec());
    in_pin.options = Some(
        PinOptions::new()
            .set_valid_values(vec!["a".into(), "b".into()])
            .build(),
    );
    connect(&mut out_pin, &mut in_pin);

    let producer = node_with_pins("n_producer", "producer", vec![out_pin]);
    let consumer = node_with_pins("n_consumer", "consumer", vec![in_pin]);

    let mut board = empty_board();
    for node in [producer, consumer] {
        board.nodes.insert(node.id.clone(), node);
    }
    let mut variable = Variable::new("my_var", VariableType::String, ValueType::Normal);
    variable.default_value = Some(b"\"hello\"".to_vec());
    board.variables.insert(variable.id.clone(), variable);
    board
        .refs
        .insert("ref_a".to_string(), "{\"type\":\"string\"}".to_string());

    let compiled = compile_board(&board).expect("compile");
    let fingerprint = [7u8; 32];
    let bytes = encode_artifact(&compiled, &fingerprint).expect("encode");

    let header = peek_header(&bytes).expect("header");
    assert_eq!(header.registry_fingerprint, fingerprint);

    let decoded = decode_artifact(&bytes, Some(&fingerprint)).expect("decode");
    assert_eq!(decoded, compiled);

    assert!(
        decode_artifact(&bytes, Some(&[9u8; 32])).is_err(),
        "wrong registry fingerprint must be rejected"
    );
}

/// A reroute whose input has no upstream but carries a default literal acts
/// as a value source; splicing it away would drop the literal. It must be
/// kept as an ordinary node.
#[test]
fn dangling_reroute_with_default_is_kept() {
    let mut r_in = pin("r_in", "route_in", PinType::Input, VariableType::String, 0);
    r_in.default_value = Some(b"\"literal\"".to_vec());
    let mut r_out = pin(
        "r_out",
        "route_out",
        PinType::Output,
        VariableType::String,
        1,
    );
    let mut in_pin = pin("p_in", "value_in", PinType::Input, VariableType::String, 0);
    connect(&mut r_out, &mut in_pin);

    let reroute = node_with_pins("n_r", "reroute", vec![r_in, r_out]);
    let consumer = node_with_pins("n_consumer", "consumer", vec![in_pin]);

    let mut board = empty_board();
    for node in [reroute, consumer] {
        board.nodes.insert(node.id.clone(), node);
    }

    let compiled = compile_board(&board).expect("compile");
    assert_eq!(compiled.nodes.len(), 2, "value-source reroute must survive");
    let consumer_in = compiled.pins.iter().find(|p| p.id == "p_in").unwrap();
    assert_eq!(
        consumer_in.depends_on.len(),
        1,
        "consumer keeps its dependency on the kept reroute"
    );
}

/// A WASM node that happens to be named "reroute" must never be spliced.
#[test]
fn wasm_reroute_lookalike_is_not_spliced() {
    let mut r_in = pin("r_in", "route_in", PinType::Input, VariableType::String, 0);
    let mut r_out = pin(
        "r_out",
        "route_out",
        PinType::Output,
        VariableType::String,
        1,
    );
    let mut out_pin = pin(
        "p_out",
        "value_out",
        PinType::Output,
        VariableType::String,
        0,
    );
    let mut in_pin = pin("p_in", "value_in", PinType::Input, VariableType::String, 0);
    connect(&mut out_pin, &mut r_in);
    connect(&mut r_out, &mut in_pin);

    let mut lookalike = node_with_pins("n_r", "reroute", vec![r_in, r_out]);
    lookalike.wasm = Some(flow_like::flow::node::NodeWasm {
        package_id: "pkg".to_string(),
        permissions: vec![],
    });
    let producer = node_with_pins("n_producer", "producer", vec![out_pin]);
    let consumer = node_with_pins("n_consumer", "consumer", vec![in_pin]);

    let mut board = empty_board();
    for node in [lookalike, producer, consumer] {
        board.nodes.insert(node.id.clone(), node);
    }

    let compiled = compile_board(&board).expect("compile");
    assert_eq!(compiled.nodes.len(), 3);
    assert_eq!(compiled.pins.len(), 4);
}

/// Structurally valid rkyv payloads with out-of-range indices must fail
/// decode (validate()) instead of panicking later in template build.
#[test]
fn out_of_range_indices_are_rejected_at_decode() {
    let producer = node_with_pins(
        "n_producer",
        "producer",
        vec![pin(
            "p_out",
            "value_out",
            PinType::Output,
            VariableType::String,
            0,
        )],
    );
    let mut board = empty_board();
    board.nodes.insert(producer.id.clone(), producer);

    let mut compiled = compile_board(&board).expect("compile");
    compiled.nodes[0].layer = 5;
    assert!(compiled.validate().is_err(), "bad layer index");
    let bytes = encode_artifact(&compiled, &[0u8; 32]).expect("encode");
    assert!(
        decode_artifact(&bytes, None).is_err(),
        "decode must reject out-of-range node.layer"
    );

    let mut compiled = compile_board(&board).expect("compile");
    compiled.pins[0].depends_on = vec![99];
    let bytes = encode_artifact(&compiled, &[0u8; 32]).expect("encode");
    assert!(
        decode_artifact(&bytes, None).is_err(),
        "decode must reject out-of-range pin edges"
    );
}

/// Catalog-default metadata is interned to `None` and reinflated
/// byte-identically; user edits always survive verbatim; ambiguous
/// (multi-pin) names are never interned.
#[test]
fn catalog_interning_roundtrips_and_keeps_user_edits() {
    use flow_like::flow::compiled::view::reconstruct_board;
    use flow_like::flow::execution::context::ExecutionContext;
    use flow_like::flow::node::NodeLogic;
    use flow_like::state::FlowNodeRegistryInner;
    use flow_like_types::async_trait;
    use std::sync::Arc;

    struct DemoLogic;

    #[async_trait]
    impl NodeLogic for DemoLogic {
        fn get_node(&self) -> Node {
            let mut node = Node::new("demo_node", "Demo", "A demo node", "Tests");
            node.add_input_pin("value_in", "Value", "The input value", VariableType::String);
            node.add_input_pin("multi", "Multi A", "First multi pin", VariableType::String);
            node.add_input_pin("multi", "Multi B", "Second multi pin", VariableType::String);
            node
        }

        async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
            Ok(())
        }
    }

    let mut registry = FlowNodeRegistryInner::new(1);
    let logic: Arc<dyn NodeLogic> = Arc::new(DemoLogic);
    registry.insert(logic.get_node(), logic);

    // Placed node: description matches the catalog, friendly_name is a user
    // rename, pins copy the catalog metadata (incl. the ambiguous multi pins).
    let default_node = registry.get_node("demo_node").expect("default");
    let mut placed = default_node.clone();
    placed.id = "n_demo".to_string();
    placed.friendly_name = "My renamed demo".to_string();

    let mut board = empty_board();
    board.nodes.insert(placed.id.clone(), placed.clone());

    let compiled =
        flow_like::flow::compiled::compile_board_with_catalog(&board, &registry).expect("compile");

    let cn = &compiled.nodes[0];
    assert_eq!(
        cn.friendly_name.as_deref(),
        Some("My renamed demo"),
        "user rename must be stored verbatim"
    );
    assert_eq!(cn.description, None, "catalog-default description interned");
    assert_eq!(cn.category, None, "catalog-default category interned");

    let value_pin = compiled
        .pins
        .iter()
        .find(|p| p.name == "value_in")
        .expect("value_in");
    assert_eq!(value_pin.friendly_name, None);
    assert_eq!(value_pin.description, None);

    for pin in compiled.pins.iter().filter(|p| p.name == "multi") {
        assert!(
            pin.friendly_name.is_some() && pin.description.is_some(),
            "ambiguous multi-pins must never be interned"
        );
    }

    // Roundtrip through the artifact and reinflate byte-identically.
    let bytes = encode_artifact(&compiled, &[3u8; 32]).expect("encode");
    let decoded = decode_artifact(&bytes, None).expect("decode");
    let view =
        reconstruct_board(&decoded, Path::from("apps").child("t"), Some(&registry)).expect("view");
    let restored = &view.nodes["n_demo"];
    assert_eq!(restored.friendly_name, "My renamed demo");
    assert_eq!(restored.description, placed.description);
    assert_eq!(restored.category, placed.category);
    for (id, pin) in &placed.pins {
        let restored_pin = &restored.pins[id];
        assert_eq!(restored_pin.friendly_name, pin.friendly_name);
        assert_eq!(restored_pin.description, pin.description);
    }

    // Without the catalog, interned fields must fail loudly, not silently
    // reconstruct empty.
    assert!(
        reconstruct_board(&decoded, Path::from("apps").child("t"), None).is_err(),
        "interned artifact without catalog must be rejected"
    );
}

#[test]
fn corrupt_artifacts_are_rejected() {
    let board = empty_board();
    let compiled = compile_board(&board).expect("compile");
    let bytes = encode_artifact(&compiled, &[0u8; 32]).expect("encode");

    assert!(peek_header(&bytes[..10]).is_err(), "short buffer");

    let mut wrong_magic = bytes.clone();
    wrong_magic[0] = b'X';
    assert!(peek_header(&wrong_magic).is_err(), "wrong magic");

    let mut wrong_version = bytes.clone();
    wrong_version[4] = 0xFF;
    wrong_version[5] = 0xFF;
    assert!(
        peek_header(&wrong_version).is_err(),
        "unknown format version"
    );

    let mut truncated = bytes.clone();
    truncated.truncate(bytes.len() - 8);
    assert!(
        decode_artifact(&truncated, None).is_err(),
        "truncated payload"
    );
}

#[test]
fn reconstructed_view_preserves_execution_fields() {
    let mut out_pin = pin(
        "p_out",
        "value_out",
        PinType::Output,
        VariableType::Struct,
        0,
    );
    out_pin.schema = Some("ref_a".to_string());
    let mut in_pin = pin("p_in", "value_in", PinType::Input, VariableType::Struct, 0);
    connect(&mut out_pin, &mut in_pin);

    let mut producer = node_with_pins("n_producer", "producer", vec![out_pin]);
    producer.friendly_name = "Renamed by user".to_string();
    let consumer = node_with_pins("n_consumer", "consumer", vec![in_pin]);

    let mut board = empty_board();
    for node in [producer, consumer] {
        board.nodes.insert(node.id.clone(), node);
    }
    board
        .refs
        .insert("ref_a".to_string(), "{\"type\":\"object\"}".to_string());

    let compiled = compile_board(&board).expect("compile");
    let view = reconstruct_board(&compiled, Path::from("apps").child("app-x"), None).expect("view");

    assert_eq!(view.id, board.id);
    assert_eq!(view.nodes.len(), 2);
    let producer_view = &view.nodes["n_producer"];
    assert_eq!(producer_view.friendly_name, "Renamed by user");
    assert_eq!(producer_view.description, "producer node");
    let out_view = &producer_view.pins["p_out"];
    assert_eq!(out_view.schema.as_deref(), Some("ref_a"));
    assert!(out_view.connected_to.contains("p_in"));
    assert_eq!(view.refs["ref_a"], "{\"type\":\"object\"}");
    assert_eq!(
        view.nodes["n_consumer"].pins["p_in"]
            .depends_on
            .iter()
            .next()
            .map(String::as_str),
        Some("p_out")
    );

    let _ = NONE_IDX;
}

/// The executor builds its template from fetched bytes and nothing else, so
/// every way those bytes can be wrong has to surface as an error it can act
/// on — never as a silent recompile, which it no longer has.
#[test]
fn template_from_bytes_rejects_foreign_or_broken_artifacts() {
    use flow_like::flow::compiled::template_from_bytes;
    use flow_like::state::FlowNodeRegistryInner;

    let mut board = empty_board();
    board
        .nodes
        .insert("n".into(), node_with_pins("n", "lonely", vec![]));
    let compiled = compile_board(&board).expect("compile");
    let registry = FlowNodeRegistryInner::new(0);
    let root = Path::from("apps").child("test");

    let compiled_with = [1u8; 32];
    let bytes = encode_artifact(&compiled, &compiled_with).expect("encode");
    let error = template_from_bytes(&bytes, &[2u8; 32], &registry, &root)
        .err()
        .expect("a foreign fingerprint is rejected before decoding");
    let message = error.to_string();
    let theirs = blake3::Hash::from_bytes(compiled_with).to_hex();
    let ours = blake3::Hash::from_bytes([2u8; 32]).to_hex();
    assert!(message.contains(&theirs.as_str()[..16]), "{message}");
    assert!(message.contains(&ours.as_str()[..16]), "{message}");

    assert!(
        template_from_bytes(b"not an artifact", &compiled_with, &registry, &root).is_err(),
        "garbage bytes are an error, not a template"
    );
}
