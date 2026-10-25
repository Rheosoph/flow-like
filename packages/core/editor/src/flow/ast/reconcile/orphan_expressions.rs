use super::*;

type Dependencies = HashMap<String, HashSet<String>>;

/// Remove pure helpers whose last consumer or backing variable this edit removes. Starting
/// from affected edges keeps unrelated canvas nodes intact, even when they are disconnected.
pub(super) fn remove_orphans(
    board: &Board,
    retained_anchors: &HashSet<&str>,
    commands: &mut Vec<BoardCommand>,
) {
    if !commands.iter().any(|command| {
        matches!(
            command,
            BoardCommand::RemoveNode { .. } | BoardCommand::RemoveVariable { .. }
        )
    }) {
        return;
    }
    let removed: HashSet<String> = commands
        .iter()
        .filter_map(|command| match command {
            BoardCommand::RemoveNode { node_id, .. } => Some(node_id.clone()),
            _ => None,
        })
        .collect();
    let removed_variables: HashSet<&str> = commands
        .iter()
        .filter_map(|command| match command {
            BoardCommand::RemoveVariable { variable_id, .. } => Some(variable_id.as_str()),
            _ => None,
        })
        .collect();
    let index = BoardIndex::new(board);
    let pure: HashSet<String> = index
        .nodes
        .values()
        .filter(|node| {
            node.name != "reroute"
                && !is_trigger_entry(node)
                && !node.pins.values().any(is_exec_pin)
                && !removed.contains(&node.id)
        })
        .map(|node| node.id.clone())
        .collect();
    if pure.is_empty() {
        return;
    }

    let target_pin = |owner: &str, reference: &str| -> Option<&Pin> {
        if let Some(node) = index.nodes.get(owner) {
            return find_input_pin_by_ref(node, reference);
        }
        let layer = board.layers.get(owner)?;
        layer.pins.get(reference).or_else(|| {
            find_boundary_pin_by_ref(&layer.pins, reference, PinType::Output)
                .or_else(|| find_boundary_pin_by_ref(&layer.pins, reference, PinType::Input))
        })
    };
    let source_owner = |owner: &str, reference: &str| -> String {
        let pin = index
            .nodes
            .get(owner)
            .and_then(|node| {
                node.pins
                    .get(reference)
                    .or_else(|| find_output_pin(node, reference))
            })
            .or_else(|| {
                let layer = board.layers.get(owner)?;
                layer.pins.get(reference).or_else(|| {
                    find_boundary_pin_by_ref(&layer.pins, reference, PinType::Output).or_else(
                        || find_boundary_pin_by_ref(&layer.pins, reference, PinType::Input),
                    )
                })
            });
        pin.and_then(|pin| index.data_source_for_pin_id(&pin.id))
            .map(|source| source.node.node_ref())
            .unwrap_or_else(|| owner.to_string())
    };

    let mut replaced_inputs = HashSet::new();
    let mut disconnected: HashMap<(String, String), HashSet<String>> = HashMap::new();
    let mut planned_sources = HashSet::new();
    for command in commands.iter() {
        match command {
            BoardCommand::ConnectPins {
                from_node,
                from_pin,
                to_node,
                to_pin,
                ..
            } => {
                // New consumers do not have live pins yet. Their existing producers must stay.
                planned_sources.insert(source_owner(from_node, from_pin));
                if let Some(pin) = target_pin(to_node, to_pin)
                    && !is_exec_pin(pin)
                {
                    replaced_inputs.insert((to_node.clone(), pin.id.clone()));
                }
            }
            BoardCommand::DisconnectPins {
                from_node,
                from_pin,
                to_node,
                to_pin,
                ..
            } => {
                if let Some(pin) = target_pin(to_node, to_pin)
                    && !is_exec_pin(pin)
                {
                    disconnected
                        .entry((to_node.clone(), pin.id.clone()))
                        .or_default()
                        .insert(source_owner(from_node, from_pin));
                }
            }
            _ => {}
        }
    }

    let mut before = Dependencies::new();
    let mut after = Dependencies::new();
    let mut add_input = |owner: &str, pin: &Pin| {
        if is_exec_pin(pin) {
            return;
        }
        let key = (owner.to_string(), pin.id.clone());
        // Input dependencies are authoritative. A replaced input can leave a stale backlink
        // on its previous producer until the board's pin cleanup runs.
        for source in index.data_sources_for_pin(pin) {
            let source = source.node.node_ref();
            before
                .entry(owner.to_string())
                .or_default()
                .insert(source.clone());
            if !removed.contains(owner)
                && !removed.contains(&source)
                && !replaced_inputs.contains(&key)
                && !disconnected
                    .get(&key)
                    .is_some_and(|sources| sources.contains(&source))
            {
                after.entry(owner.to_string()).or_default().insert(source);
            }
        }
    };
    for node in index.nodes.values().filter(|node| node.name != "reroute") {
        for pin in node
            .pins
            .values()
            .filter(|pin| pin.pin_type == PinType::Input)
        {
            add_input(&node.id, pin);
        }
    }
    // Presentational boundaries are traversed by BoardIndex. Function returns are live sinks.
    for layer in board
        .layers
        .values()
        .filter(|layer| matches!(layer.r#type, LayerType::Function))
    {
        for pin in layer
            .pins
            .values()
            .filter(|pin| pin.pin_type == PinType::Output)
        {
            add_input(&layer.id, pin);
        }
    }

    let mut pending = Vec::new();
    for (consumer, sources) in &before {
        if removed.contains(consumer) {
            pending.extend(sources.iter().cloned());
        }
    }
    if !removed_variables.is_empty() {
        let mut consumers = Dependencies::new();
        for (consumer, sources) in &before {
            for source in sources {
                consumers
                    .entry(source.clone())
                    .or_default()
                    .insert(consumer.clone());
            }
        }
        let mut readers: Vec<String> = index
            .nodes
            .values()
            .filter_map(|node| {
                (node.name == "variable_get"
                    && node_pin_literal_string(node, "var_ref")
                        .is_some_and(|id| removed_variables.contains(id.as_str())))
                .then(|| node.id.clone())
            })
            .collect();
        let mut visited = HashSet::new();
        while let Some(id) = readers.pop() {
            if !pure.contains(&id) || !visited.insert(id.clone()) {
                continue;
            }
            // A disconnected chain of pure helpers cannot keep a deleted variable alive.
            // Surviving statements and explicit anchors still protect the chain below.
            if let Some(downstream) = consumers.get(&id) {
                readers.extend(downstream.iter().cloned());
            }
            pending.push(id);
        }
    }
    let mut candidates = HashSet::new();
    while let Some(id) = pending.pop() {
        if !pure.contains(&id) || !candidates.insert(id.clone()) {
            continue;
        }
        if let Some(sources) = before.get(&id) {
            pending.extend(sources.iter().cloned());
        }
    }

    // Trace backwards from surviving consumers. This also preserves shared subexpressions and
    // can discard an affected cycle when no surviving consumer needs any node in that cycle.
    let mut pending: Vec<String> = after
        .keys()
        .filter(|owner| !candidates.contains(*owner))
        .cloned()
        .chain(retained_anchors.iter().map(|id| id.to_string()))
        .chain(planned_sources)
        .collect();
    let mut live = HashSet::new();
    while let Some(id) = pending.pop() {
        if !live.insert(id.clone()) {
            continue;
        }
        if let Some(sources) = after.get(&id) {
            pending.extend(sources.iter().cloned());
        }
    }
    let dead: BTreeSet<String> = candidates.difference(&live).cloned().collect();
    if dead.is_empty() {
        return;
    }
    // Removing a node already removes all of its edges, so omit redundant disconnects.
    commands.retain(|command| {
        !matches!(command, BoardCommand::DisconnectPins { from_node, to_node, .. }
            if dead.contains(from_node) || dead.contains(to_node))
    });
    commands.extend(dead.into_iter().map(|id| BoardCommand::RemoveNode {
        summary: Some(format!(
            "Remove unused {}",
            index.nodes[id.as_str()].friendly_name
        )),
        node_id: id,
    }));
}
