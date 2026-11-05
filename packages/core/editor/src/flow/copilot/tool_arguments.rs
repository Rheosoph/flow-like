use serde_json::Value;

fn rename_aliases(value: &mut Value, canonical: &str, aliases: &[&str]) -> Result<(), String> {
    let Some(object) = value.as_object_mut() else {
        return Ok(());
    };
    let present = std::iter::once(canonical)
        .chain(aliases.iter().copied())
        .filter(|key| object.contains_key(*key))
        .collect::<Vec<_>>();
    if present.len() > 1 {
        return Err(format!(
            "Conflicting arguments {} refer to {canonical}; provide only {canonical}.",
            present.join(", ")
        ));
    }
    if let Some(alias) = present.first().copied().filter(|key| *key != canonical)
        && let Some(value) = object.remove(alias)
    {
        object.insert(canonical.to_owned(), value);
    }
    Ok(())
}

/// Preserve the explicit Serde compatibility forms before validating the canonical tool schema.
/// Unknown fields and arbitrary payloads stay intact so validation can still reject them.
pub(super) fn canonicalize(name: &str, arguments: &mut Value) -> Result<(), String> {
    let aliases: &[(&str, &[&str])] = match name {
        "execute_event" => &[
            ("app_id", &["appId"]),
            ("event_id", &["eventId"]),
            ("stream_state", &["streamState"]),
        ],
        "execute_node" => &[
            ("app_id", &["appId"]),
            ("board_id", &["boardId"]),
            ("node_id", &["nodeId"]),
            ("stream_state", &["streamState"]),
        ],
        "query_execution_logs" => &[
            ("app_id", &["appId"]),
            ("board_id", &["boardId"]),
            ("run_id", &["runId"]),
            ("filter", &["query"]),
            ("run_metadata", &["runMetadata"]),
        ],
        "run_board_tests" => &[
            ("app_id", &["appId"]),
            ("board_id", &["boardId"]),
            ("max_tests", &["maxTests"]),
        ],
        _ => &[],
    };
    for (canonical, aliases) in aliases {
        rename_aliases(arguments, canonical, aliases)?;
    }
    Ok(())
}
