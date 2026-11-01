use super::*;

#[derive(Default)]
struct ConfigEdit {
    value: Option<flow_like_types::Value>,
    authored_inputs: HashSet<String>,
}

/// A changed literal configuration retires the wires into placeholders it no longer declares.
/// Unchanged configurations retire only omitted inputs, so stale documents still round-trip.
pub(super) fn disconnect_removed_inputs<'a>(
    board: &Board,
    calls: impl IntoIterator<Item = (&'a str, &'a Call)>,
    result: &mut ReconcileResult,
) {
    let mut edits = BTreeMap::<String, ConfigEdit>::new();
    for (id, call) in calls {
        let Some(node) = find_board_node(board, id) else {
            continue;
        };
        let Some(config_name) = dynamic_placeholder_config_pin(&node.name) else {
            continue;
        };
        let meta = node_to_metadata(node);
        let Ok(call) = fold_call_arguments(call, &meta, live_receiver_pin(&meta).as_deref()) else {
            continue;
        };
        // Shared pure nodes can appear once per consumer. Preserve every authored input
        // across those projections, rather than depending on traversal order.
        let edit = edits.entry(id.to_string()).or_default();
        for arg in &call.args {
            for pin in matching_input_pins(node, &arg.name) {
                edit.authored_inputs.insert(pin.id.clone());
                if pin.name == config_name {
                    edit.value = literal_expr_to_value(&arg.value);
                }
            }
        }
    }

    // Inlined calls can reuse a live node without carrying their own anchor. Their planned
    // config writes are authoritative too. The last write determines the applied template.
    for command in &result.commands {
        let (id, pin_ref, value) = match command {
            BoardCommand::UpdateNodePin {
                node_id,
                pin_id,
                value,
                ..
            } => (node_id, pin_id, Some(value.clone())),
            BoardCommand::DisconnectPins {
                to_node, to_pin, ..
            } => (to_node, to_pin, None),
            _ => continue,
        };
        let Some(node) = find_board_node(board, id) else {
            continue;
        };
        let Some(config_name) = dynamic_placeholder_config_pin(&node.name) else {
            continue;
        };
        if !find_input_pin_by_ref(node, pin_ref).is_some_and(|pin| pin.name == config_name) {
            continue;
        }
        let edit = edits.entry(id.clone()).or_default();
        if let Some(value) = value {
            edit.value = Some(value);
        } else if edit.value.is_none() {
            // Replacing a wired config with its stored literal needs only a disconnect.
            edit.value = find_input_pin(node, config_name)
                .and_then(|pin| pin.default_value.as_deref())
                .and_then(|bytes| flow_like_types::json::from_slice(bytes).ok());
        }
    }
    if edits.is_empty() {
        return;
    }

    let index = BoardIndex::new(board);
    let removed: HashSet<&str> = result
        .commands
        .iter()
        .filter_map(|command| match command {
            BoardCommand::RemoveNode { node_id, .. } => Some(node_id.as_str()),
            _ => None,
        })
        .collect();
    let mut disconnects = Vec::new();
    for (id, edit) in edits {
        if removed.contains(id.as_str()) {
            continue;
        }
        let Some(node) = find_board_node(board, &id) else {
            continue;
        };
        let Some(config_name) = dynamic_placeholder_config_pin(&node.name) else {
            continue;
        };
        let Some(config) = find_input_pin(node, config_name) else {
            continue;
        };
        let Some(template) = edit.value.as_ref().and_then(|value| value.as_str()) else {
            continue;
        };
        let config_changed = config
            .default_value
            .as_deref()
            .and_then(|bytes| flow_like_types::json::from_slice::<String>(bytes).ok())
            .as_deref()
            != Some(template);
        let Some(declared) = dynamic_template_placeholders(&node.name, template) else {
            continue;
        };
        // SQL on_update preserves its pins when two names map to the same FlowScript argument.
        if sql_param_node(&node.name) {
            let unique: HashSet<_> = declared.iter().map(|name| to_camel_case(name)).collect();
            if unique.len() != declared.len() {
                continue;
            }
        }
        let source_owner = |pin_id: &str| {
            index
                .pin_owner
                .get(pin_id)
                .map(|(owner, _)| owner.id.clone())
                .or_else(|| {
                    index
                        .boundary_sources
                        .get(pin_id)
                        .map(|source| source.node.node_ref())
                })
        };
        let targets = |target: &str, reference: &str, pin: &Pin| {
            target == id && find_input_pin_by_ref(node, reference).is_some_and(|p| p.id == pin.id)
        };
        let disconnected = |pin: &Pin, source: &str| {
            result.commands.iter().any(|command| {
                matches!(command,
                    BoardCommand::DisconnectPins { from_pin, to_node, to_pin, .. }
                        if from_pin == source && targets(to_node, to_pin, pin)
                )
            })
        };
        let connected = |pin: &Pin| {
            result.commands.iter().any(|command| matches!(command,
                BoardCommand::ConnectPins { to_node, to_pin, .. } if targets(to_node, to_pin, pin)
            ))
        };
        if connected(config)
            || config.depends_on.iter().any(|source| {
                !disconnected(config, source)
                    && !source_owner(source).is_some_and(|owner| removed.contains(owner.as_str()))
            })
        {
            continue;
        }

        let mut stale: Vec<_> = node
            .pins
            .values()
            .filter(|pin| {
                pin.pin_type == PinType::Input
                    && pin.name != config_name
                    && !is_exec_pin(pin)
                    && (!sql_param_node(&node.name)
                        || flow_like_storage::databases::sql_params::placeholder_from_pin_name(
                            &pin.name,
                        )
                        .is_some())
                    && !declared.contains(&pin.name)
                    && (config_changed || !edit.authored_inputs.contains(&pin.id))
            })
            .collect();
        stale.sort_by(|a, b| a.id.cmp(&b.id));
        for pin in stale {
            if connected(pin) {
                result.diagnostics.push(format!(
                    "input `{}` on `{}` is no longer declared by `{config_name}`; remove its argument or restore the placeholder",
                    pin.name, node.friendly_name
                ));
                continue;
            }
            let mut sources: Vec<_> = pin.depends_on.iter().collect();
            sources.sort();
            for source in sources {
                let Some(owner) = source_owner(source) else {
                    continue;
                };
                if removed.contains(owner.as_str()) || disconnected(pin, source) {
                    continue;
                }
                disconnects.push(BoardCommand::DisconnectPins {
                    from_node: owner,
                    from_pin: source.clone(),
                    to_node: id.clone(),
                    to_pin: pin.id.clone(),
                    summary: Some(format!(
                        "Disconnect undeclared placeholder {} on {}",
                        pin.name, node.friendly_name
                    )),
                });
            }
        }
    }
    result.commands.extend(disconnects);
}
