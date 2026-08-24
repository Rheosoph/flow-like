use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{self, Write},
    path::PathBuf,
    process::ExitCode,
    sync::Arc,
    time::UNIX_EPOCH,
};

use clap::Parser;
use flow_like::{
    flow::{
        ast::{RenderOptions, apply_flowscript_to_board, board_to_flowscript, format_flowscript},
        board::Board,
    },
    state::{FlowLikeConfig, FlowLikeState},
    utils::http::HTTPClient,
};
use flow_like_storage::Path;
use flow_like_types::tokio;
use regex::Regex;
use serde::Serialize;
use serde_json::Value;

const OUTPUT_SCHEMA: &str = "flow-like.flowscript-render-data/v1";
const ERROR_SCHEMA: &str = "flow-like.flowscript-render-data-error/v1";

#[derive(Parser)]
#[command(
    about = "Reconcile FlowScript with the real catalog and emit browser-renderable board data"
)]
struct Cli {
    /// FlowScript source file to reconcile.
    input: PathBuf,

    /// Stable board id used by the screenshot fixture.
    #[arg(long, default_value = "flowscript-render-board")]
    board_id: String,

    /// Board title shown in Studio. Defaults to the source file stem.
    #[arg(long)]
    name: Option<String>,
}

#[derive(Serialize)]
struct RenderData {
    schema: &'static str,
    board: Board,
    catalog: Vec<flow_like::flow::node::Node>,
    canonical_flowscript: String,
}

#[derive(Debug, Serialize)]
struct RenderError {
    schema: &'static str,
    error: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    diagnostics: Vec<String>,
}

fn write_json(value: &impl Serialize) -> io::Result<()> {
    let stdout = io::stdout();
    let mut lock = stdout.lock();
    serde_json::to_writer(&mut lock, value)?;
    lock.write_all(b"\n")
}

fn source_name(input: &std::path::Path) -> String {
    input
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("FlowScript Workflow")
        .to_string()
}

fn coordinate_key(coordinates: Option<(f32, f32, f32)>) -> String {
    coordinates
        .map(|(x, y, z)| {
            format!(
                "{:08x}:{:08x}:{:08x}",
                x.to_bits(),
                y.to_bits(),
                z.to_bits()
            )
        })
        .unwrap_or_else(|| "none".to_string())
}

fn pin_signature(pins: &HashMap<String, flow_like::flow::pin::Pin>) -> String {
    let mut pins = pins
        .values()
        .map(|pin| {
            format!(
                "{:05}:{:?}:{}:{:?}:{:?}",
                pin.index, pin.pin_type, pin.name, pin.data_type, pin.value_type
            )
        })
        .collect::<Vec<_>>();
    pins.sort();
    pins.join("|")
}

fn layer_semantic_path(
    board: &Board,
    id: &str,
    cache: &mut HashMap<String, String>,
    visiting: &mut HashSet<String>,
) -> String {
    if let Some(path) = cache.get(id) {
        return path.clone();
    }
    if !visiting.insert(id.to_string()) {
        return "<cycle>".to_string();
    }
    let path = match board.layers.get(id) {
        Some(layer) => {
            let parent = layer
                .parent_id
                .as_deref()
                .filter(|parent| !parent.is_empty())
                .map(|parent| layer_semantic_path(board, parent, cache, visiting))
                .unwrap_or_else(|| "<root>".to_string());
            format!(
                "{parent}/{:?}:{}:{}",
                layer.r#type,
                layer.name,
                coordinate_key(Some(layer.coordinates))
            )
        }
        None => "<missing>".to_string(),
    };
    visiting.remove(id);
    cache.insert(id.to_string(), path.clone());
    path
}

fn layer_semantic_paths(board: &Board) -> HashMap<String, String> {
    let mut result = HashMap::new();
    for id in board.layers.keys() {
        layer_semantic_path(board, id, &mut result, &mut HashSet::new());
    }
    result
}

fn anchor_pairs(text: &str) -> Vec<(String, String)> {
    let anchor = Regex::new(r"//@([nlv]):([^\s]+)").expect("static FlowScript anchor regex");
    anchor
        .captures_iter(text)
        .map(|capture| (capture[1].to_string(), capture[2].to_string()))
        .collect()
}

fn normalized_anchor_text(text: &str) -> String {
    let anchor = Regex::new(r"//@([nlv]):([^\s]+)").expect("static FlowScript anchor regex");
    anchor.replace_all(text, "//@${1}:<id>").into_owned()
}

fn is_safe_preserved_anchor(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && !matches!(id, "__proto__" | "prototype" | "constructor")
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

/// Preserve Studio-authored identities when the imported source is exactly the canonical board
/// text modulo ids. Hand-authored or partially anchored documents fall back to render-only ids.
fn preferred_anchor_ids(source: &str, generated: &str) -> HashMap<String, String> {
    let Ok(source) = format_flowscript(source, true) else {
        return HashMap::new();
    };
    if normalized_anchor_text(&source) != normalized_anchor_text(generated) {
        return HashMap::new();
    }
    let source = anchor_pairs(&source);
    let generated = anchor_pairs(generated);
    if source.len() != generated.len() {
        return HashMap::new();
    }

    generated
        .into_iter()
        .zip(source)
        .filter_map(
            |((generated_kind, generated_id), (source_kind, source_id))| {
                (generated_kind == source_kind).then_some((generated_id, source_id))
            },
        )
        .collect()
}

fn assign_stable_id(
    old_id: &str,
    prefix: &str,
    ordinal: usize,
    preferred: &HashMap<String, String>,
    translations: &mut HashMap<String, String>,
    used: &mut HashSet<String>,
) {
    if translations.contains_key(old_id) {
        return;
    }
    let preferred_id = preferred
        .get(old_id)
        .filter(|id| is_safe_preserved_anchor(id) && !used.contains(*id))
        .cloned();
    let mut id = preferred_id.unwrap_or_else(|| format!("render-{prefix}-{ordinal:04}"));
    let mut collision = 1;
    while used.contains(&id) {
        id = format!("render-{prefix}-{ordinal:04}-{collision}");
        collision += 1;
    }
    used.insert(id.clone());
    translations.insert(old_id.to_string(), id);
}

fn replace_ids(value: &mut Value, translations: &HashMap<String, String>) {
    match value {
        Value::String(text) => {
            if let Some(replacement) = translations.get(text) {
                *text = replacement.clone();
            }
        }
        Value::Array(values) => {
            for value in values {
                replace_ids(value, translations);
            }
        }
        Value::Object(values) => {
            let entries = std::mem::take(values);
            for (key, mut value) in entries {
                replace_ids(&mut value, translations);
                values.insert(translations.get(&key).cloned().unwrap_or(key), value);
            }
        }
        _ => {}
    }
}

fn rewrite_default_bytes(bytes: &mut Option<Vec<u8>>, translations: &HashMap<String, String>) {
    let Some(bytes) = bytes else {
        return;
    };
    let Ok(mut value) = serde_json::from_slice::<Value>(bytes) else {
        return;
    };
    replace_ids(&mut value, translations);
    if let Ok(rewritten) = serde_json::to_vec(&value) {
        *bytes = rewritten;
    }
}

fn stabilize_board_ids(
    board: &mut Board,
    source: &str,
    generated_flowscript: &str,
) -> Result<(), serde_json::Error> {
    let preferred = preferred_anchor_ids(source, generated_flowscript);
    let layer_paths = layer_semantic_paths(board);
    let mut translations = HashMap::new();
    let mut used = HashSet::new();

    let mut layer_ids = board.layers.keys().cloned().collect::<Vec<_>>();
    layer_ids.sort_by(|left, right| {
        layer_paths
            .get(left)
            .cmp(&layer_paths.get(right))
            .then(left.cmp(right))
    });
    for (index, id) in layer_ids.iter().enumerate() {
        assign_stable_id(
            id,
            "layer",
            index + 1,
            &preferred,
            &mut translations,
            &mut used,
        );
    }

    let node_key = |node: &flow_like::flow::node::Node| {
        format!(
            "{}|{}|{}|{}|{}|{}",
            node.layer
                .as_deref()
                .and_then(|id| layer_paths.get(id))
                .cloned()
                .unwrap_or_else(|| "<root>".to_string()),
            coordinate_key(node.coordinates),
            node.start.unwrap_or(false),
            node.name,
            node.friendly_name,
            pin_signature(&node.pins)
        )
    };
    let mut nodes = board
        .nodes
        .values()
        .map(|node| (node_key(node), node.id.clone()))
        .collect::<Vec<_>>();
    for (layer_id, layer) in &board.layers {
        for node in layer.nodes.values() {
            if !board.nodes.contains_key(&node.id) {
                nodes.push((
                    format!(
                        "{}|{}",
                        layer_paths
                            .get(layer_id)
                            .cloned()
                            .unwrap_or_else(|| "<missing>".to_string()),
                        node_key(node)
                    ),
                    node.id.clone(),
                ));
            }
        }
    }
    nodes.sort();
    for (index, (_, id)) in nodes.iter().enumerate() {
        assign_stable_id(
            id,
            "node",
            index + 1,
            &preferred,
            &mut translations,
            &mut used,
        );
    }

    let mut variables = board
        .variables
        .values()
        .map(|variable| {
            (
                format!(
                    "<root>|{}|{:?}|{:?}",
                    variable.name, variable.data_type, variable.value_type
                ),
                variable.id.clone(),
            )
        })
        .collect::<Vec<_>>();
    for (layer_id, layer) in &board.layers {
        for variable in layer.variables.values() {
            variables.push((
                format!(
                    "{}|{}|{:?}|{:?}",
                    layer_paths
                        .get(layer_id)
                        .cloned()
                        .unwrap_or_else(|| "<missing>".to_string()),
                    variable.name,
                    variable.data_type,
                    variable.value_type
                ),
                variable.id.clone(),
            ));
        }
    }
    variables.sort();
    for (index, (_, id)) in variables.iter().enumerate() {
        assign_stable_id(
            id,
            "variable",
            index + 1,
            &preferred,
            &mut translations,
            &mut used,
        );
    }

    let mut comments = board
        .comments
        .values()
        .map(|comment| {
            (
                format!(
                    "<root>|{}|{}",
                    coordinate_key(Some(comment.coordinates)),
                    comment.content
                ),
                comment.id.clone(),
            )
        })
        .collect::<Vec<_>>();
    for (layer_id, layer) in &board.layers {
        for comment in layer.comments.values() {
            comments.push((
                format!(
                    "{}|{}|{}",
                    layer_paths
                        .get(layer_id)
                        .cloned()
                        .unwrap_or_else(|| "<missing>".to_string()),
                    coordinate_key(Some(comment.coordinates)),
                    comment.content
                ),
                comment.id.clone(),
            ));
        }
    }
    comments.sort();
    for (index, (_, id)) in comments.iter().enumerate() {
        assign_stable_id(
            id,
            "comment",
            index + 1,
            &preferred,
            &mut translations,
            &mut used,
        );
    }

    let mut pins = Vec::new();
    let mut add_pins = |owner: String, values: &HashMap<String, flow_like::flow::pin::Pin>| {
        for pin in values.values() {
            pins.push((
                format!(
                    "{}|{:05}|{:?}|{}|{:?}|{:?}",
                    owner, pin.index, pin.pin_type, pin.name, pin.data_type, pin.value_type
                ),
                pin.id.clone(),
            ));
        }
    };
    for node in board.nodes.values() {
        add_pins(
            translations
                .get(&node.id)
                .cloned()
                .unwrap_or_else(|| node.id.clone()),
            &node.pins,
        );
    }
    for layer in board.layers.values() {
        let layer_owner = translations
            .get(&layer.id)
            .cloned()
            .unwrap_or_else(|| layer.id.clone());
        add_pins(layer_owner.clone(), &layer.pins);
        for node in layer.nodes.values() {
            add_pins(
                translations
                    .get(&node.id)
                    .cloned()
                    .unwrap_or_else(|| format!("{layer_owner}/{}", node.name)),
                &node.pins,
            );
        }
    }
    pins.sort();
    for (index, (_, id)) in pins.iter().enumerate() {
        assign_stable_id(
            id,
            "pin",
            index + 1,
            &HashMap::new(),
            &mut translations,
            &mut used,
        );
    }

    // Embedded JSON defaults carry function/layer ids as bytes; rewrite them before the ordinary
    // recursive JSON pass handles map keys and direct string references.
    for node in board.nodes.values_mut() {
        node.hash = None;
        for pin in node.pins.values_mut() {
            rewrite_default_bytes(&mut pin.default_value, &translations);
        }
    }
    for variable in board.variables.values_mut() {
        variable.hash = None;
        rewrite_default_bytes(&mut variable.default_value, &translations);
    }
    for comment in board.comments.values_mut() {
        comment.hash = None;
        comment.timestamp = UNIX_EPOCH;
    }
    for layer in board.layers.values_mut() {
        layer.hash = None;
        for pin in layer.pins.values_mut() {
            rewrite_default_bytes(&mut pin.default_value, &translations);
        }
        for node in layer.nodes.values_mut() {
            node.hash = None;
            for pin in node.pins.values_mut() {
                rewrite_default_bytes(&mut pin.default_value, &translations);
            }
        }
        for variable in layer.variables.values_mut() {
            variable.hash = None;
            rewrite_default_bytes(&mut variable.default_value, &translations);
        }
        for comment in layer.comments.values_mut() {
            comment.hash = None;
            comment.timestamp = UNIX_EPOCH;
        }
    }
    board.hash = None;
    board.created_at = UNIX_EPOCH;
    board.updated_at = UNIX_EPOCH;

    let mut value = serde_json::to_value(&*board)?;
    replace_ids(&mut value, &translations);
    *board = serde_json::from_value(value)?;
    Ok(())
}

async fn reconcile(args: &Cli) -> Result<RenderData, RenderError> {
    let source = fs::read_to_string(&args.input).map_err(|error| RenderError {
        schema: ERROR_SCHEMA,
        error: format!("Failed to read {}: {error}", args.input.display()),
        diagnostics: Vec::new(),
    })?;

    reconcile_source(
        &source,
        &args.board_id,
        args.name
            .clone()
            .unwrap_or_else(|| source_name(&args.input)),
        format!("Rendered from {}", args.input.display()),
    )
    .await
}

async fn reconcile_source(
    source: &str,
    board_id: &str,
    board_name: String,
    board_description: String,
) -> Result<RenderData, RenderError> {
    // FlowScript emitted by Studio carries stable identity anchors for editing an existing board.
    // This command imports into a new ephemeral board, where those identities cannot exist yet.
    // Canonicalize without anchors first, then use the ordinary reconcile/apply path. The resulting
    // board receives fresh identities and `canonical_flowscript` below exposes their new anchors.
    let import_source = format_flowscript(source, false).map_err(|error| RenderError {
        schema: ERROR_SCHEMA,
        error: format!("FlowScript parse failed: {error}"),
        diagnostics: Vec::new(),
    })?;

    let state = Arc::new(FlowLikeState::new(
        FlowLikeConfig::new(),
        HTTPClient::new_without_refetch(),
    ));
    let catalog_logic = flow_like_catalog::get_catalog();
    let catalog_nodes = catalog_logic
        .iter()
        .map(|logic| logic.get_node())
        .collect::<Vec<_>>();
    state.node_registry.write().await.push_nodes(catalog_logic);

    let mut board = Board::new_detached(Some(board_id.to_string()), Path::default());
    board.name = board_name;
    board.description = board_description;

    let applied = apply_flowscript_to_board(
        &mut board,
        &import_source,
        &catalog_nodes,
        state,
        None,
        false,
    )
    .await
    .map_err(|error| RenderError {
        schema: ERROR_SCHEMA,
        error: format!("FlowScript reconciliation failed: {error}"),
        diagnostics: Vec::new(),
    })?;

    if !applied.diagnostics.is_empty() {
        return Err(RenderError {
            schema: ERROR_SCHEMA,
            error: "FlowScript did not reconcile cleanly".to_string(),
            diagnostics: applied.diagnostics,
        });
    }
    if board.nodes.is_empty() && board.layers.is_empty() {
        return Err(RenderError {
            schema: ERROR_SCHEMA,
            error: "FlowScript reconciled without producing any renderable nodes or layers"
                .to_string(),
            diagnostics: Vec::new(),
        });
    }

    let generated_flowscript = board_to_flowscript(
        &board,
        &RenderOptions {
            anchors: true,
            ..RenderOptions::default()
        },
    );
    stabilize_board_ids(&mut board, source, &generated_flowscript).map_err(|error| {
        RenderError {
            schema: ERROR_SCHEMA,
            error: format!("Failed to stabilize reconciled workflow identities: {error}"),
            diagnostics: Vec::new(),
        }
    })?;

    let used_node_types = board
        .nodes
        .values()
        .map(|node| node.name.as_str())
        .collect::<HashSet<_>>();
    let mut used_catalog = catalog_nodes
        .into_iter()
        .filter(|node| used_node_types.contains(node.name.as_str()))
        .collect::<Vec<_>>();
    used_catalog.sort_by(|left, right| left.name.cmp(&right.name));

    let canonical_flowscript = board_to_flowscript(
        &board,
        &RenderOptions {
            anchors: true,
            ..RenderOptions::default()
        },
    );

    Ok(RenderData {
        schema: OUTPUT_SCHEMA,
        board,
        catalog: used_catalog,
        canonical_flowscript,
    })
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = Cli::parse();
    match reconcile(&args).await {
        Ok(output) => match write_json(&output) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("Failed to write render data: {error}");
                ExitCode::from(2)
            }
        },
        Err(error) => {
            if let Err(write_error) = write_json(&error) {
                eprintln!("{}; failed to write error JSON: {write_error}", error.error);
            }
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn imports_studio_anchors_into_a_fresh_board() {
        let source = r#"
eventsGeneric anchored(payload: Struct, report: string) { //@n:old-event-id
    log::info({ message: report, toast: false }) //@n:old-info-id
}
"#;
        let rendered = reconcile_source(
            source,
            "anchored-test-board",
            "Anchored".to_string(),
            String::new(),
        )
        .await
        .expect("anchored Studio FlowScript should import into an empty board");

        assert!(!rendered.board.nodes.is_empty());
        assert!(rendered.canonical_flowscript.contains("//@n:"));
        assert!(
            rendered.canonical_flowscript.contains("old-event-id"),
            "source canonical:\n{}\nrendered canonical:\n{}",
            format_flowscript(source, true).expect("format source"),
            rendered.canonical_flowscript
        );
        assert!(rendered.canonical_flowscript.contains("old-info-id"));

        let repeated = reconcile_source(
            source,
            "anchored-test-board",
            "Anchored".to_string(),
            String::new(),
        )
        .await
        .expect("a repeated import should also reconcile");
        assert_eq!(
            serde_json::to_value(&rendered.board).expect("serialize first board"),
            serde_json::to_value(&repeated.board).expect("serialize repeated board"),
            "the same source must produce identical ids and positions"
        );
        assert_eq!(rendered.canonical_flowscript, repeated.canonical_flowscript);
    }

    #[tokio::test]
    async fn unanchored_import_ids_are_repeatable() {
        let source = r#"
use log::*

eventsGeneric repeatable(payload: Struct, report: string) {
    info({ message: report, toast: false })
}
"#;
        let first = reconcile_source(
            source,
            "repeatable-test-board",
            "Repeatable".to_string(),
            String::new(),
        )
        .await
        .expect("first unanchored import should reconcile");
        let second = reconcile_source(
            source,
            "repeatable-test-board",
            "Repeatable".to_string(),
            String::new(),
        )
        .await
        .expect("second unanchored import should reconcile");

        assert!(
            first
                .board
                .nodes
                .keys()
                .all(|id| id.starts_with("render-node-"))
        );
        assert_eq!(
            serde_json::to_value(&first.board).expect("serialize first board"),
            serde_json::to_value(&second.board).expect("serialize second board"),
            "unanchored source must produce identical ids, pins, and positions"
        );
        assert_eq!(first.canonical_flowscript, second.canonical_flowscript);
    }

    #[test]
    fn unsafe_source_anchors_are_never_preserved_as_object_keys_or_paths() {
        assert!(!is_safe_preserved_anchor("__proto__"));
        assert!(!is_safe_preserved_anchor("constructor"));
        assert!(!is_safe_preserved_anchor("parent/child"));
        assert!(!is_safe_preserved_anchor("node id"));
        assert!(is_safe_preserved_anchor("ck-safe_node-01"));
    }
}
