use super::*;

fn placeholder_board(node_type: &str, template: &str, inputs: &[&str]) -> Board {
    let mut board = empty_board();
    let mut source = Node::new("text_source", "Source", "", "test");
    source.id = "source".into();
    let output = source
        .add_output_pin("value", "Value", "", VariableType::String)
        .id
        .clone();
    board.nodes.insert(source.id.clone(), source);

    let config = dynamic_placeholder_config_pin(node_type).expect("placeholder node");
    let mut target = Node::new(node_type, "Target", "", "test");
    target.id = "target".into();
    target.set_receiver(config);
    target
        .add_input_pin(config, "Configuration", "", VariableType::String)
        .default_value = Some(flow_like_types::json::to_vec(template).unwrap());
    target.add_output_pin("result", "Result", "", VariableType::String);
    let pins = inputs
        .iter()
        .map(|name| {
            target
                .add_input_pin(name, name, "", VariableType::Generic)
                .id
                .clone()
        })
        .collect::<Vec<_>>();
    board.nodes.insert(target.id.clone(), target);
    for input in pins {
        connect(&mut board, "source", &output, "target", &input);
    }
    board
}

fn placeholder_call(call: &str) -> String {
    format!("detached {{\n    {call} //@n:target\n}}")
}

fn cleanup_plan(board: &Board, call: Option<&str>, commands: Vec<BoardCommand>) -> ReconcileResult {
    let ast = flow_like_ast::parse(&call.map(placeholder_call).unwrap_or_default())
        .expect("valid FlowScript fixture");
    let mut calls = HashMap::new();
    collect_calls(&ast, &mut calls);
    let mut result = ReconcileResult {
        commands,
        ..Default::default()
    };
    placeholder_pins::disconnect_removed_inputs(
        board,
        calls.iter().map(|(id, call)| (id.as_str(), *call)),
        &mut result,
    );
    result
}

fn disconnected_names<'a>(board: &'a Board, result: &ReconcileResult) -> BTreeSet<&'a str> {
    result
        .commands
        .iter()
        .filter_map(|command| match command {
            BoardCommand::DisconnectPins {
                to_node, to_pin, ..
            } if to_node == "target" => {
                find_input_pin_by_ref(&board.nodes["target"], to_pin).map(|pin| pin.name.as_str())
            }
            _ => None,
        })
        .collect()
}

fn config_write(board: &Board, value: flow_like_types::Value) -> BoardCommand {
    BoardCommand::UpdateNodePin {
        node_id: "target".into(),
        pin_id: dynamic_placeholder_config_pin(&board.nodes["target"].name)
            .unwrap()
            .into(),
        value,
        summary: None,
    }
}

#[test]
fn identical_config_repairs_omitted_stale_placeholders_without_catalog() {
    let board = placeholder_board(
        "string_format",
        "{search} {ledger}",
        &["search", "ledger", "completion", "stats"],
    );
    let result = reconcile_text(
        &board,
        &placeholder_call(
            "stringFormat({ formatString: \"{search} {ledger}\", search: shared, ledger: shared })",
        ),
    );
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    assert_eq!(
        disconnected_names(&board, &result),
        BTreeSet::from(["completion", "stats"])
    );
    assert!(!result.commands.iter().any(|command| matches!(
        command,
        BoardCommand::UpdateNodePin { pin_id, .. } if pin_id == "format_string"
    )));
}

#[test]
fn explicitly_authored_stale_inputs_survive_an_unchanged_document() {
    let board = placeholder_board("string_format", "{search}", &["search", "stats"]);
    let result = reconcile_text(
        &board,
        &placeholder_call(
            "stringFormat({ formatString: \"{search}\", search: shared, stats: shared })",
        ),
    );
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    assert!(result.commands.is_empty(), "{:?}", result.commands);
}

#[test]
fn receiver_and_friendly_config_spelling_authorize_placeholder_cleanup() {
    let board = placeholder_board("string_format", "{search} {stats}", &["search", "stats"]);
    for call in [
        "\"{search}\".format({ search: shared })",
        "stringFormat({ configuration: \"{search}\", search: shared })",
    ] {
        let result = cleanup_plan(&board, Some(call), Vec::new());
        assert_eq!(
            disconnected_names(&board, &result),
            BTreeSet::from(["stats"])
        );
    }
}

#[test]
fn a_changed_literal_retires_placeholders_even_when_old_arguments_remain() {
    let board = placeholder_board("string_format", "{search} {stats}", &["search", "stats"]);
    let result = cleanup_plan(
        &board,
        Some("stringFormat({ formatString: \"{search}\", search: shared, stats: shared })"),
        vec![config_write(&board, "{search}".into())],
    );
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    assert_eq!(
        disconnected_names(&board, &result),
        BTreeSet::from(["stats"])
    );
}

#[test]
fn unreadable_or_ambiguous_templates_preserve_existing_connections() {
    for (node_type, old, replacement, input) in [
        ("string_render_template", "{{ stats }}", "{{", "stats"),
        ("df_sql_query", "SELECT $stats", "SELECT '", "param_stats"),
        ("filter_local_db", "id = $stats", "id = '", "param_stats"),
        (
            "df_sql_query",
            "SELECT $stats",
            "SELECT $foo_bar, $fooBar",
            "param_stats",
        ),
    ] {
        let board = placeholder_board(node_type, old, &[input]);
        let result = cleanup_plan(&board, None, vec![config_write(&board, replacement.into())]);
        assert!(
            disconnected_names(&board, &result).is_empty(),
            "{node_type}: {replacement:?}: {:?}",
            result.commands
        );
    }
}

#[test]
fn missing_nonliteral_and_nonstring_configs_preserve_existing_connections() {
    let board = placeholder_board("string_format", "{stats}", &["stats"]);
    for call in [
        "stringFormat({})",
        "stringFormat({ formatString: runtimeTemplate })",
        "stringFormat({ formatString: null })",
        "stringFormat({ formatString: 7 })",
    ] {
        let result = cleanup_plan(&board, Some(call), Vec::new());
        assert!(
            disconnected_names(&board, &result).is_empty(),
            "{call}: {:?}",
            result.commands
        );
    }
}

#[test]
fn a_wired_configuration_preserves_placeholders_until_its_edge_is_replaced() {
    let mut board = placeholder_board("string_format", "{search}", &["search", "stats"]);
    let source = find_output_pin(&board.nodes["source"], "value")
        .unwrap()
        .id
        .clone();
    let config = find_input_pin(&board.nodes["target"], "format_string")
        .unwrap()
        .id
        .clone();
    connect(&mut board, "source", &source, "target", &config);
    let call = "stringFormat({ formatString: \"{search}\", search: shared })";
    let unchanged = cleanup_plan(&board, Some(call), Vec::new());
    assert!(disconnected_names(&board, &unchanged).is_empty());

    let replaced = cleanup_plan(
        &board,
        Some(call),
        vec![BoardCommand::DisconnectPins {
            from_node: "source".into(),
            from_pin: source,
            to_node: "target".into(),
            to_pin: config,
            summary: None,
        }],
    );
    assert_eq!(
        disconnected_names(&board, &replaced),
        BTreeSet::from(["format_string", "stats"])
    );
}

#[test]
fn a_new_config_connection_keeps_placeholders_despite_a_stored_literal_write() {
    let board = placeholder_board("string_format", "{stats}", &["stats"]);
    let result = cleanup_plan(
        &board,
        None,
        vec![
            config_write(&board, "no placeholders".into()),
            BoardCommand::ConnectPins {
                from_node: "source".into(),
                from_pin: "value".into(),
                to_node: "target".into(),
                to_pin: "formatString".into(),
                summary: None,
            },
        ],
    );
    assert!(disconnected_names(&board, &result).is_empty());
}

#[test]
fn sql_cleanup_preserves_static_inputs_for_query_and_filter_nodes() {
    for (node_type, template) in [
        ("df_sql_query", "SELECT $search"),
        ("filter_local_db", "id = $search"),
    ] {
        let board = placeholder_board(
            node_type,
            template,
            &["param_search", "param_stats", "params", "session", "param_"],
        );
        let result = cleanup_plan(&board, None, vec![config_write(&board, template.into())]);
        assert_eq!(
            disconnected_names(&board, &result),
            BTreeSet::from(["param_stats"]),
            "{node_type}: {:?}",
            result.commands
        );
    }
}

#[test]
fn placeholder_named_format_string_in_camel_case_is_still_dynamic() {
    let board = placeholder_board("string_format", "{formatString}", &["formatString"]);
    let result = cleanup_plan(
        &board,
        None,
        vec![config_write(&board, "plain text".into())],
    );
    assert_eq!(
        disconnected_names(&board, &result),
        BTreeSet::from(["formatString"])
    );
}

#[test]
fn the_final_config_write_controls_removal_for_reused_calls() {
    let board = placeholder_board("string_format", "{search} {stats}", &["search", "stats"]);
    let result = cleanup_plan(
        &board,
        None,
        vec![
            config_write(&board, "{stats}".into()),
            config_write(&board, "{search}".into()),
        ],
    );
    assert_eq!(
        disconnected_names(&board, &result),
        BTreeSet::from(["stats"])
    );
}

#[test]
fn cleanup_does_not_repeat_disconnects_or_target_removed_endpoints() {
    let board = placeholder_board("string_format", "{stats}", &["stats"]);
    let source = find_output_pin(&board.nodes["source"], "value")
        .unwrap()
        .id
        .clone();
    let input = find_input_pin(&board.nodes["target"], "stats")
        .unwrap()
        .id
        .clone();
    for reference in [input, "stats".into()] {
        let result = cleanup_plan(
            &board,
            None,
            vec![
                config_write(&board, "plain text".into()),
                BoardCommand::DisconnectPins {
                    from_node: "source".into(),
                    from_pin: source.clone(),
                    to_node: "target".into(),
                    to_pin: reference,
                    summary: None,
                },
            ],
        );
        assert_eq!(result.commands.len(), 2, "{:?}", result.commands);
    }
    for removed in ["source", "target"] {
        let result = cleanup_plan(
            &board,
            None,
            vec![
                config_write(&board, "plain text".into()),
                BoardCommand::RemoveNode {
                    node_id: removed.into(),
                    summary: None,
                },
            ],
        );
        assert!(
            disconnected_names(&board, &result).is_empty(),
            "removing {removed}: {:?}",
            result.commands
        );
    }
}
