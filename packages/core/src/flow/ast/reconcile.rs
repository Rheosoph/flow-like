//! Reconcile: parsed FlowScript (`BoardAst`) → `BoardCommand`s against an existing board.
//!
//! This is the *write* half of the FlowScript pipeline. Rendering lowers a [`Board`] into a
//! [`BoardAst`] and emits text (with optional `//@n:<id>` anchors); editing that text and parsing
//! it back yields a new `BoardAst`. Reconcile diffs the new AST against the live board and emits
//! the minimal set of [`BoardCommand`]s needed to realize the edit — reusing the exact command
//! enum the copilot already validates and applies (so undo/redo, coordinates, and all
//! side-channel metadata of untouched nodes survive). See `todo/ast.md` §7.
//!
//! ## Scope
//!
//! The plain [`reconcile`] path targets the dominant "edit an existing board through text"
//! operations, which are fully and unambiguously reconstructable:
//!
//! - **Configuration edits**: a literal argument on an anchored call changed ⇒ [`BoardCommand::UpdateNodePin`].
//! - **Deletions**: a *text-visible* anchored node present in the board but absent from the new
//!   AST ⇒ [`BoardCommand::RemoveNode`].
//!
//! The catalog-aware [`reconcile_with_catalog`] path additionally turns new unanchored catalog
//! calls into structural commands (`AddNode`, `ConnectPins`, `UpdateNodePin`). Reconcile still
//! never guesses identities or ambiguous catalog matches: a partial or ambiguous edit produces
//! diagnostics instead of destructive commands.

use std::collections::{HashMap, HashSet};

use flow_like_ast::model::*;
use flow_like_ast::to_camel_case;

use crate::flow::board::Board;
use crate::flow::copilot::{
    BoardCommand, NodeMetadata, NodePosition, PinMetadata, PlaceholderPinDef, node_to_metadata,
};
use crate::flow::node::Node;
use crate::flow::pin::{Pin, PinType};
use crate::flow::variable::VariableType;

/// Outcome of reconciling a parsed `BoardAst` against a live board.
#[derive(Debug, Default, Clone)]
pub struct ReconcileResult {
    /// Minimal board mutations to realize the edit, in apply order.
    pub commands: Vec<BoardCommand>,
    /// Non-fatal notes (e.g. an anchor that no longer resolves to a board node, or a structural
    /// change that reconcile deliberately left to the node-authoring tools).
    pub diagnostics: Vec<String>,
}

/// Diff a parsed FlowScript AST against `existing` and emit the minimal `BoardCommand`s.
///
/// Only nodes carrying a stable anchor (`//@n:<id>`) are eligible for in-place edits or removal,
/// and removal is further gated on the node being *text-visible* (it appears in the board's own
/// lowered/rendered form) so inlined/sugared helper nodes are never deleted merely for being
/// absent from the text. See the module docs for the full contract.
pub fn reconcile(existing: &Board, new: &BoardAst) -> ReconcileResult {
    reconcile_inner(existing, new, None, None)
}

/// Diff a parsed FlowScript AST against `existing`, using catalog metadata to turn unanchored
/// calls into structural commands.
///
/// The non-catalog [`reconcile`] path remains intentionally conservative for callers that only
/// want anchored edits. FlowPilot should use this catalog-aware path so new text-domain calls can
/// become `AddNode` / `ConnectPins` / `UpdateNodePin` command batches automatically.
pub fn reconcile_with_catalog(
    existing: &Board,
    new: &BoardAst,
    catalog: &[NodeMetadata],
) -> ReconcileResult {
    reconcile_inner(existing, new, Some(catalog), None)
}

/// Like [`reconcile_with_catalog`] but runs `enricher` to materialize each new node's dynamic
/// (`on_update`-generated) pins from its literal args, so literals/connections targeting those pins
/// resolve against real pins instead of the predicted `synthesize_dynamic_input_pin` fallback.
pub fn reconcile_with_catalog_enriched(
    existing: &Board,
    new: &BoardAst,
    catalog: &[NodeMetadata],
    enricher: &MetadataEnricher,
) -> ReconcileResult {
    reconcile_inner(existing, new, Some(catalog), Some(enricher))
}

/// Hook that lets a caller enrich a resolved node's metadata with the dynamic pins its `on_update`
/// would create for a call's literal arguments (see `apply_flowscript_to_board`). Runs only for
/// callers that supply one; `None` keeps the pure static-catalog behavior (predicted via
/// `synthesize_dynamic_input_pin`).
pub type MetadataEnricher = Box<
    dyn Fn(&NodeMetadata, &[(String, flow_like_types::Value)], &Board) -> Option<NodeMetadata>
        + Send
        + Sync,
>;

fn reconcile_inner(
    existing: &Board,
    new: &BoardAst,
    catalog: Option<&[NodeMetadata]>,
    enricher: Option<&MetadataEnricher>,
) -> ReconcileResult {
    let mut result = ReconcileResult::default();
    let board_ast = super::lower_to_ast(existing);
    let variable_refs = VariableRefLookup::from_board_and_ast(existing, new);

    let variable_changes = reconcile_variables(&board_ast, new);
    result.commands.extend(variable_changes.commands);
    result.diagnostics.extend(variable_changes.diagnostics);

    // Anchored `base.path = value` writes carry no `&Call`, so synthesize the equivalent
    // `struct_set` calls the anchor-keyed collectors below diff against / delete. These arenas own
    // the calls and must outlive the `new_calls`/`visible` maps that borrow them, so build them in
    // full up front — pushing to the Vec later would reallocate and invalidate the references.
    let new_field_assign_calls = collect_anchored_field_assign_calls(new);
    let board_field_assign_calls = collect_anchored_field_assign_calls(&board_ast);

    // 1. Index every anchored call in the new AST by node id.
    let mut new_calls: HashMap<String, &Call> = HashMap::new();
    collect_calls(new, &mut new_calls);
    for call in &new_field_assign_calls {
        if let Some(anchor) = call.anchor.as_deref() {
            new_calls.entry(anchor.to_string()).or_insert(call);
        }
    }

    // 2. Configuration edits: for each anchored call that still maps to a board node, diff its
    //    literal arguments against the node's current pin defaults.
    for (anchor, call) in &new_calls {
        let Some(node) = find_board_node(existing, anchor) else {
            result.diagnostics.push(format!(
                "anchor {anchor} no longer resolves to a board node; skipped"
            ));
            continue;
        };
        // Multi-pins (several input pins sharing one name) pair positionally with the
        // same-named args, mirroring the order lowering emitted them in.
        let mut same_name_seen: HashMap<&str, usize> = HashMap::new();
        for arg in &call.args {
            let occurrence = {
                let counter = same_name_seen.entry(arg.name.as_str()).or_insert(0);
                let index = *counter;
                *counter += 1;
                index
            };
            let Some(mut new_value) = literal_expr_to_value(&arg.value) else {
                // References / nested calls describe wiring, which v1 does not rewrite.
                continue;
            };
            let pins = matching_input_pins(node, &arg.name);
            let Some(pin) = pins.get(occurrence).copied() else {
                // No pin at this occurrence: a literal on a pin the node's `on_update` will mint
                // (e.g. a placeholder added to an existing `string_format`) is deferred to apply,
                // which creates the pin and then applies the write; a genuinely unknown pin stays a
                // (non-fatal) skip.
                if arg_targets_predicted_dynamic_pin(node, anchor, call, arg, existing, enricher) {
                    normalize_variable_ref_value_for_pin(&mut new_value, &arg.name, &variable_refs);
                    result.commands.push(BoardCommand::UpdateNodePin {
                        node_id: anchor.clone(),
                        pin_id: arg.name.clone(),
                        value: new_value,
                        summary: Some(format!("Set {} on {}", arg.name, node.friendly_name)),
                    });
                } else {
                    result.diagnostics.push(format!(
                        "node {anchor} has no input pin named {:?} (occurrence {}); skipped",
                        arg.name,
                        occurrence + 1
                    ));
                }
                continue;
            };
            normalize_variable_ref_value_for_pin(&mut new_value, &pin.name, &variable_refs);
            let current = pin
                .default_value
                .as_deref()
                .and_then(|b| flow_like_types::json::from_slice::<flow_like_types::Value>(b).ok());
            if current.as_ref() == Some(&new_value) {
                continue; // unchanged
            }
            result.commands.push(BoardCommand::UpdateNodePin {
                node_id: anchor.clone(),
                // The name is ambiguous across multi-pins; address those by exact pin id.
                pin_id: if pins.len() > 1 {
                    pin.id.clone()
                } else {
                    pin.name.clone()
                },
                value: new_value,
                summary: Some(format!("Set {} on {}", arg.name, node.friendly_name)),
            });
        }
    }

    // 3. Deletions: a text-visible anchored node absent from the new AST is a removal. We compute
    //    "text-visible" from the board's own lowered AST so sugared/inlined nodes (reroutes,
    //    struct_make, pure helpers) are never removed just for lacking an anchor in the text.
    let mut visible: HashMap<String, &Call> = HashMap::new();
    collect_statement_calls(&board_ast, &mut visible);
    for call in &board_field_assign_calls {
        if let Some(anchor) = call.anchor.as_deref() {
            visible.entry(anchor.to_string()).or_insert(call);
        }
    }
    let new_anchors: HashSet<&String> = new_calls.keys().collect();
    let mut removed_ids: HashSet<String> = HashSet::new();
    for anchor in visible.keys() {
        if new_anchors.contains(anchor) {
            continue;
        }
        let Some(node) = find_board_node(existing, anchor) else {
            continue;
        };
        removed_ids.insert(anchor.clone());
        result.commands.push(BoardCommand::RemoveNode {
            node_id: anchor.clone(),
            summary: Some(format!("Remove {}", node.friendly_name)),
        });
    }

    // 4. Structural authoring: catalog-aware FlowPilot calls can add new unanchored calls in the
    //    text. Translate those to the same command format the UI already reviews/applies.
    if let Some(catalog) = catalog {
        let structural = StructuralPlanner::new(existing, catalog, enricher).plan(new);
        if !structural.commands.is_empty() {
            let anchored_edits = std::mem::take(&mut result.commands);
            result.commands = structural.commands;
            result.commands.extend(anchored_edits);
        }
        result.diagnostics.extend(structural.diagnostics);
    } else if ast_has_unanchored_calls(new) {
        result.diagnostics.push(
            "FlowScript contains new unanchored calls; catalog metadata is required to turn them into board commands."
                .to_string(),
        );
    }

    // Bridge severed exec chains AFTER structural planning merged: when a removed statement was
    // REPLACED, the planner already re-targeted the predecessor's exec output (exec outputs are
    // single-target replace at apply), so a bridge from that same source would steal the new
    // node's wiring and orphan it. Skip sources the merged plan already drives.
    if !removed_ids.is_empty() {
        let occupied_sources: HashSet<(String, String)> = result
            .commands
            .iter()
            .filter_map(|command| match command {
                BoardCommand::ConnectPins {
                    from_node,
                    from_pin,
                    ..
                } => Some((from_node.clone(), from_pin.clone())),
                _ => None,
            })
            .collect();
        bridge_removed_exec_chains(
            existing,
            &removed_ids,
            &occupied_sources,
            &mut result.commands,
            &mut result.diagnostics,
        );
    }

    // Layer-size gate LAST, over the merged command set: a violating edit queues nothing.
    if let Some(violations) = layer_node_limit_violations(existing, &result.commands) {
        result.commands.clear();
        result.diagnostics.extend(violations);
    }

    result
}

/// Re-join execution chains severed by deletions: for each removed segment whose predecessor and
/// (transitive) successor survive, emit the bridging ConnectPins so the tail keeps running.
/// Layer boundary pins count as endpoints — deleting the first statement of a function body
/// re-joins its layer's `exec_in` to the surviving successor. Sources the merged plan already
/// drives (`occupied_sources`) are skipped: the planner's replacement wiring wins. Ambiguous
/// shapes (multiple predecessors or successors) get a diagnostic instead of a guess.
fn bridge_removed_exec_chains(
    existing: &Board,
    removed: &HashSet<String>,
    occupied_sources: &HashSet<(String, String)>,
    commands: &mut Vec<BoardCommand>,
    diagnostics: &mut Vec<String>,
) {
    // pin id → (owner ref for commands, pin, walkable node when the owner is a node).
    let mut pin_owner: HashMap<&str, (String, &Pin, Option<&Node>)> = HashMap::new();
    for node in all_board_nodes(existing) {
        for pin in node.pins.values() {
            pin_owner.insert(pin.id.as_str(), (node.id.clone(), pin, Some(node)));
        }
    }
    for layer in existing.layers.values() {
        for pin in layer.pins.values() {
            pin_owner.insert(pin.id.as_str(), (layer.id.clone(), pin, None));
        }
    }

    for node_id in removed {
        let Some(node) = find_board_node(existing, node_id) else {
            continue;
        };
        // Only bridge from a segment head: its predecessor survives the edit. Mid-segment
        // removed nodes are covered by their head's forward walk.
        let mut preds: Vec<(String, String)> = node
            .pins
            .values()
            .filter(|pin| pin.pin_type == PinType::Input && is_exec_pin(pin))
            .flat_map(|pin| pin.depends_on.iter())
            .filter_map(|pin_id| pin_owner.get(pin_id.as_str()))
            .filter(|(owner_id, _, _)| !removed.contains(owner_id))
            .map(|(owner_id, pin, _)| (owner_id.clone(), pin.name.clone()))
            .collect();
        preds.sort();
        preds.dedup();
        let [(pred_node, pred_pin)] = preds.as_slice() else {
            continue;
        };
        if occupied_sources.contains(&(pred_node.clone(), pred_pin.clone())) {
            continue;
        }

        let mut frontier = vec![node];
        let mut visited: HashSet<String> = HashSet::new();
        let mut successors: Vec<(String, String)> = Vec::new();
        while let Some(current) = frontier.pop() {
            if !visited.insert(current.id.clone()) {
                continue;
            }
            for pin in current
                .pins
                .values()
                .filter(|pin| pin.pin_type == PinType::Output && is_exec_pin(pin))
            {
                for target_pin_id in &pin.connected_to {
                    let Some((owner_id, target_pin, owner_node)) =
                        pin_owner.get(target_pin_id.as_str())
                    else {
                        continue;
                    };
                    if removed.contains(owner_id) {
                        if let Some(owner_node) = owner_node {
                            frontier.push(owner_node);
                        }
                    } else {
                        successors.push((owner_id.clone(), target_pin.name.clone()));
                    }
                }
            }
        }
        successors.sort();
        successors.dedup();

        match successors.as_slice() {
            [] => {}
            [(succ_node, succ_pin)] => {
                commands.push(BoardCommand::ConnectPins {
                    from_node: pred_node.clone(),
                    from_pin: pred_pin.clone(),
                    to_node: succ_node.clone(),
                    to_pin: succ_pin.clone(),
                    summary: Some(format!(
                        "Re-join execution around removed {}",
                        node.friendly_name
                    )),
                });
            }
            _ => diagnostics.push(format!(
                "removing `{}` leaves multiple execution successors; the chain after it was not re-joined automatically — reconnect the intended successor explicitly",
                node.friendly_name
            )),
        }
    }
}

/// Enforce [`MAX_NODES_PER_LAYER`]: existing per-layer populations plus the edit's additions
/// (minus its removals) must stay within the cap. Returns violation diagnostics, or `None` when
/// the edit fits.
fn layer_node_limit_violations(
    existing: &Board,
    commands: &[BoardCommand],
) -> Option<Vec<String>> {
    let mut counts: HashMap<Option<String>, i64> = HashMap::new();
    let mut layer_names: HashMap<String, String> = HashMap::new();
    let mut node_layers: HashMap<&str, Option<String>> = HashMap::new();

    for node in existing.nodes.values() {
        *counts.entry(node.layer.clone()).or_default() += 1;
        node_layers.insert(node.id.as_str(), node.layer.clone());
    }
    for layer in existing.layers.values() {
        layer_names.insert(layer.id.clone(), layer.name.clone());
        for node in layer.nodes.values() {
            *counts.entry(Some(layer.id.clone())).or_default() += 1;
            node_layers.insert(node.id.as_str(), Some(layer.id.clone()));
        }
    }

    for command in commands {
        match command {
            BoardCommand::AddNode { target_layer, .. } => {
                *counts.entry(target_layer.clone()).or_default() += 1;
            }
            BoardCommand::CreateLayer {
                ref_id: Some(ref_id),
                name,
                ..
            } => {
                layer_names.insert(ref_id.clone(), name.clone());
            }
            BoardCommand::RemoveNode { node_id, .. } => {
                if let Some(layer) = node_layers.get(node_id.as_str()) {
                    *counts.entry(layer.clone()).or_default() -= 1;
                }
            }
            _ => {}
        }
    }

    let mut violations: Vec<String> = counts
        .iter()
        .filter(|(_, count)| **count > MAX_NODES_PER_LAYER as i64)
        .map(|(layer, count)| {
            let scope = match layer {
                Some(id) => format!(
                    "function/layer `{}`",
                    layer_names.get(id).cloned().unwrap_or_else(|| id.clone())
                ),
                None => "the root layer".to_string(),
            };
            format!(
                "this edit would leave {scope} with {count} nodes (max {MAX_NODES_PER_LAYER}). Nothing was queued. Split the logic into smaller `function name(...) {{ ... }}` declarations — each function layer has its own {MAX_NODES_PER_LAYER}-node budget — and call them instead"
            )
        })
        .collect();
    violations.sort();
    (!violations.is_empty()).then_some(violations)
}

/// Convert an AST [`Literal`] into the JSON value a [`BoardCommand::UpdateNodePin`] carries.
fn literal_to_value(lit: &Literal) -> flow_like_types::Value {
    use flow_like_types::Value;
    match lit {
        Literal::String(s) => Value::String(s.clone()),
        Literal::Int(i) => Value::from(*i),
        Literal::Float(f) => Value::from(*f),
        Literal::Bool(b) => Value::Bool(*b),
        Literal::Null => Value::Null,
        Literal::Json(raw) => {
            flow_like_types::json::from_str(raw).unwrap_or_else(|_| Value::String(raw.clone()))
        }
    }
}

fn reconcile_variables(existing: &BoardAst, new: &BoardAst) -> ReconcileResult {
    let mut result = ReconcileResult::default();
    let mut existing_by_anchor: HashMap<&str, &VarDecl> = HashMap::new();
    let mut existing_by_name: HashMap<&str, &VarDecl> = HashMap::new();

    for var in &existing.variables {
        if let Some(anchor) = var.anchor.as_deref() {
            existing_by_anchor.insert(anchor, var);
        }
        existing_by_name.insert(var.name.as_str(), var);
    }

    let mut seen_existing = HashSet::new();
    for var in &new.variables {
        let matched = var
            .anchor
            .as_deref()
            .and_then(|anchor| existing_by_anchor.get(anchor).copied())
            .or_else(|| existing_by_name.get(var.name.as_str()).copied());

        match matched {
            Some(old) => {
                if let Some(anchor) = old.anchor.as_deref() {
                    seen_existing.insert(anchor.to_string());
                }
                if let Some(command) = update_variable_command(existing, new, old, var) {
                    result.commands.push(command);
                }
            }
            None => {
                result.commands.push(create_variable_command(new, var));
            }
        }
    }

    for var in &existing.variables {
        let Some(anchor) = var.anchor.as_deref() else {
            continue;
        };
        if seen_existing.contains(anchor) {
            continue;
        }
        let still_present_by_name = new.variables.iter().any(|new_var| {
            new_var.anchor.as_deref() == Some(anchor)
                || (new_var.anchor.is_none() && new_var.name == var.name)
        });
        if still_present_by_name {
            continue;
        }
        result.commands.push(BoardCommand::RemoveVariable {
            variable_id: anchor.to_string(),
            summary: Some(format!("Remove variable {}", var.name)),
        });
    }

    result
}

fn create_variable_command(ast: &BoardAst, var: &VarDecl) -> BoardCommand {
    BoardCommand::CreateVariable {
        variable_id: Some(variable_id_for_decl(var)),
        name: var.name.clone(),
        data_type: variable_data_type(var).to_string(),
        value_type: variable_value_type(var).to_string(),
        default_value: variable_default_value(var),
        description: var.description.clone(),
        category: var.category.clone(),
        schema: visible_variable_schema(ast, var),
        exposed: Some(var.exposed),
        secret: Some(var.secret),
        editable: Some(var.editable),
        runtime_configured: Some(var.runtime_configured),
        target_layer: None,
        summary: Some(format!("Create variable {}", var.name)),
    }
}

fn variable_id_for_decl(var: &VarDecl) -> String {
    var.anchor
        .clone()
        .unwrap_or_else(|| generated_variable_id(&var.name))
}

fn generated_variable_id(name: &str) -> String {
    let mut out = String::from("var_");
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out == "var_" {
        out.push_str("value");
    }
    out
}

fn update_variable_command(
    old_ast: &BoardAst,
    new_ast: &BoardAst,
    old: &VarDecl,
    new: &VarDecl,
) -> Option<BoardCommand> {
    let variable_id = old.anchor.clone()?;
    let old_default = variable_default_value(old);
    let new_default = variable_default_value(new);
    let old_schema = visible_variable_schema(old_ast, old);
    let new_schema = visible_variable_schema(new_ast, new);

    let mut changed = false;

    let name = changed_option(new.name.clone(), old.name != new.name, &mut changed);
    let data_type = changed_option(
        variable_data_type(new).to_string(),
        variable_data_type(old) != variable_data_type(new),
        &mut changed,
    );
    let value_type = changed_option(
        variable_value_type(new).to_string(),
        variable_value_type(old) != variable_value_type(new),
        &mut changed,
    );
    let default_value = changed_option(
        new_default.clone().unwrap_or(flow_like_types::Value::Null),
        old_default != new_default && new_default.is_some(),
        &mut changed,
    );
    let clear_default_value = old_default.is_some() && new_default.is_none();
    changed |= clear_default_value;

    let description = changed_option(
        new.description.clone().unwrap_or_default(),
        old.description != new.description && new.description.is_some(),
        &mut changed,
    );
    let clear_description = old.description.is_some() && new.description.is_none();
    changed |= clear_description;

    let category = changed_option(
        new.category.clone().unwrap_or_default(),
        old.category != new.category && new.category.is_some(),
        &mut changed,
    );
    let clear_category = old.category.is_some() && new.category.is_none();
    changed |= clear_category;

    let schema_changed =
        old_schema != new_schema && !schemas_structurally_equivalent(old_ast, new_ast, old, new);
    let schema = changed_option(
        new_schema.clone().unwrap_or_default(),
        schema_changed && new_schema.is_some(),
        &mut changed,
    );
    let clear_schema = schema_changed && old_schema.is_some() && new_schema.is_none();
    changed |= clear_schema;

    let exposed = changed_option(new.exposed, old.exposed != new.exposed, &mut changed);
    let secret = changed_option(new.secret, old.secret != new.secret, &mut changed);
    let editable = changed_option(new.editable, old.editable != new.editable, &mut changed);
    let runtime_configured = changed_option(
        new.runtime_configured,
        old.runtime_configured != new.runtime_configured,
        &mut changed,
    );

    changed.then_some(BoardCommand::UpdateVariable {
        variable_id,
        name,
        data_type,
        value_type,
        default_value,
        clear_default_value,
        description,
        clear_description,
        category,
        clear_category,
        schema,
        clear_schema,
        exposed,
        secret,
        editable,
        runtime_configured,
        value: None,
        summary: Some(format!("Update variable {}", new.name)),
    })
}

fn changed_option<T>(value: T, did_change: bool, changed: &mut bool) -> Option<T> {
    if did_change {
        *changed = true;
        Some(value)
    } else {
        None
    }
}

fn variable_default_value(var: &VarDecl) -> Option<flow_like_types::Value> {
    var.default.as_ref().map(literal_to_value)
}

fn variable_data_type(var: &VarDecl) -> &'static str {
    if var.schema.is_some() {
        return "Struct";
    }
    match var.ty.base.as_str() {
        "exec" | "Execution" => "Execution",
        "string" | "String" => "String",
        "int" | "Integer" => "Integer",
        "float" | "Float" => "Float",
        "bool" | "Boolean" => "Boolean",
        "Date" => "Date",
        "Path" | "PathBuf" => "PathBuf",
        "any" | "Generic" => "Generic",
        "Struct" => "Struct",
        "bytes" | "Byte" => "Byte",
        _ => "Struct",
    }
}

fn variable_value_type(var: &VarDecl) -> &'static str {
    match var.ty.container {
        Container::Normal => "Normal",
        Container::Array => "Array",
        Container::Map => "HashMap",
        Container::Set => "HashSet",
    }
}

/// Boundary execution pin names on function layers, mirrored onto `control_call_function` call
/// nodes by that node's `on_update`.
const FUNCTION_EXEC_IN: &str = "exec_in";
const FUNCTION_EXEC_OUT: &str = "exec_out";
/// The catalog node that invokes a function layer (`function_layer_id` holds the target layer).
const CALL_FUNCTION_NODE_TYPE: &str = "control_call_function";
const FUNCTION_LAYER_ID_PIN: &str = "function_layer_id";

/// Hard ceiling on nodes per layer (root, event scope, or one function layer). Oversized layers
/// make boards unreadable; reconcile rejects edits that would exceed it so the agent splits the
/// work into function layers instead.
pub const MAX_NODES_PER_LAYER: usize = 50;

fn function_layer_pins(func: &FnDecl, impure: bool) -> Vec<LayerPinMetadata> {
    let exec_pins = impure
        .then(|| {
            vec![
                LayerPinMetadata {
                    name: FUNCTION_EXEC_IN.to_string(),
                    friendly_name: "Exec In".to_string(),
                    data_type: "Execution".to_string(),
                    value_type: "Normal".to_string(),
                    pin_type: "Input".to_string(),
                },
                LayerPinMetadata {
                    name: FUNCTION_EXEC_OUT.to_string(),
                    friendly_name: "Exec Out".to_string(),
                    data_type: "Execution".to_string(),
                    value_type: "Normal".to_string(),
                    pin_type: "Output".to_string(),
                },
            ]
        })
        .unwrap_or_default();
    exec_pins
        .into_iter()
        .chain(func.params.iter().map(|param| layer_pin_from_param(param, "Input")))
        .chain(
            func.returns
                .iter()
                .map(|param| layer_pin_from_param(param, "Output")),
        )
        .collect()
}

fn execution_pin_metadata(name: &str, friendly_name: &str) -> PinMetadata {
    PinMetadata {
        name: name.to_string(),
        friendly_name: friendly_name.to_string(),
        description: String::new(),
        data_type: "Execution".to_string(),
        value_type: "Normal".to_string(),
        default_value: None,
        schema: None,
        is_generic: false,
        valid_values: None,
        enforce_schema: false,
    }
}

fn param_pin_metadata(param: &Param) -> PinMetadata {
    PinMetadata {
        name: param.name.clone(),
        friendly_name: param.name.clone(),
        description: String::new(),
        data_type: type_ref_data_type(&param.ty).to_string(),
        value_type: type_ref_value_type(&param.ty).to_string(),
        default_value: None,
        schema: None,
        is_generic: false,
        valid_values: None,
        enforce_schema: false,
    }
}

/// A FlowScript `function` declaration resolved to its (existing or planned) layer, captured
/// before events/bodies are planned so call sites anywhere in the document can target it.
#[derive(Clone)]
struct PlannedFunction {
    entity: NodeEntity,
    impure: bool,
    params: Vec<Param>,
    returns: Vec<Param>,
}

fn layer_pin_from_param(param: &Param, pin_type: &str) -> LayerPinMetadata {
    LayerPinMetadata {
        name: param.name.clone(),
        friendly_name: param.name.clone(),
        data_type: type_ref_data_type(&param.ty).to_string(),
        value_type: type_ref_value_type(&param.ty).to_string(),
        pin_type: pin_type.to_string(),
    }
}

fn type_ref_data_type(ty: &TypeRef) -> &'static str {
    match ty.base.as_str() {
        "exec" | "Execution" => "Execution",
        "string" | "String" => "String",
        "int" | "Integer" => "Integer",
        "float" | "Float" => "Float",
        "bool" | "Boolean" => "Boolean",
        "Date" => "Date",
        "Path" | "PathBuf" => "PathBuf",
        "any" | "Generic" => "Generic",
        "Struct" => "Struct",
        "bytes" | "Byte" => "Byte",
        _ => "Struct",
    }
}

fn type_ref_value_type(ty: &TypeRef) -> &'static str {
    match ty.container {
        Container::Normal => "Normal",
        Container::Array => "Array",
        Container::Map => "HashMap",
        Container::Set => "HashSet",
    }
}

fn visible_variable_schema(ast: &BoardAst, var: &VarDecl) -> Option<String> {
    let schema = var.schema.as_deref()?;
    let interface_name = flow_like_ast::interface_name_for_schema(&ast.interfaces, schema)?;
    let interface = ast
        .interfaces
        .iter()
        .find(|interface| interface.name == interface_name)?;
    flow_like_ast::schema_from_interface_with_defs(interface, &ast.interfaces)
        .or_else(|| Some(schema.to_string()))
}

fn schemas_structurally_equivalent(
    old_ast: &BoardAst,
    new_ast: &BoardAst,
    old: &VarDecl,
    new: &VarDecl,
) -> bool {
    let Some(old_schema) = comparable_variable_schema(old_ast, old) else {
        return comparable_variable_schema(new_ast, new).is_none();
    };
    comparable_variable_schema(new_ast, new).as_deref() == Some(old_schema.as_str())
}

fn comparable_variable_schema(ast: &BoardAst, var: &VarDecl) -> Option<String> {
    let schema = visible_variable_schema(ast, var).or_else(|| var.schema.clone())?;
    flow_like_ast::normalize_schema(&schema)
}

fn find_board_node<'a>(board: &'a Board, node_id: &str) -> Option<&'a Node> {
    board.nodes.get(node_id).or_else(|| {
        board
            .layers
            .values()
            .find_map(|layer| layer.nodes.get(node_id))
    })
}

fn all_board_nodes(board: &Board) -> Vec<&Node> {
    let mut nodes: Vec<&Node> = board.nodes.values().collect();
    for layer in board.layers.values() {
        nodes.extend(layer.nodes.values());
    }
    nodes
}

fn literal_expr_to_value(expr: &Expr) -> Option<flow_like_types::Value> {
    use flow_like_types::Value;
    match expr {
        Expr::Literal(lit) => Some(literal_to_value(lit)),
        Expr::Object(fields) => {
            let mut map = serde_json::Map::new();
            for field in fields {
                map.insert(field.key.clone(), literal_expr_to_value(&field.value)?);
            }
            Some(Value::Object(map))
        }
        Expr::Array(items) => {
            let mut values = Vec::with_capacity(items.len());
            for item in items {
                values.push(literal_expr_to_value(item)?);
            }
            Some(Value::Array(values))
        }
        _ => None,
    }
}

#[derive(Debug, Default, Clone)]
struct VariableRefLookup {
    by_ref: HashMap<String, String>,
}

impl VariableRefLookup {
    fn from_board(board: &Board) -> Self {
        let mut lookup = Self::default();
        for variable in board.variables.values() {
            lookup.insert(&variable.id, &variable.name);
        }
        for layer in board.layers.values() {
            for variable in layer.variables.values() {
                lookup.insert(&variable.id, &variable.name);
            }
        }
        lookup
    }

    fn from_board_and_ast(board: &Board, ast: &BoardAst) -> Self {
        let mut lookup = Self::from_board(board);
        for variable in &ast.variables {
            let id = variable_id_for_decl_against_board(board, variable);
            lookup.insert(&id, &variable.name);
        }
        lookup
    }

    fn insert(&mut self, id: &str, name: &str) {
        self.by_ref.insert(id.to_string(), id.to_string());
        self.by_ref.insert(name.to_string(), id.to_string());
        self.by_ref.insert(to_camel_case(name), id.to_string());
    }

    fn resolve(&self, value: &str) -> Option<String> {
        self.by_ref
            .get(value)
            .cloned()
            .or_else(|| self.by_ref.get(&to_camel_case(value)).cloned())
    }
}

fn variable_id_for_decl_against_board(board: &Board, var: &VarDecl) -> String {
    if let Some(anchor) = var.anchor.as_deref() {
        return anchor.to_string();
    }

    if let Some(existing) = board
        .variables
        .values()
        .find(|existing| existing.name == var.name)
    {
        return existing.id.clone();
    }

    for layer in board.layers.values() {
        if let Some(existing) = layer
            .variables
            .values()
            .find(|existing| existing.name == var.name)
        {
            return existing.id.clone();
        }
    }

    variable_id_for_decl(var)
}

fn is_variable_ref_pin_name(name: &str) -> bool {
    pin_name_matches(name, "var_ref") || pin_name_matches(name, "varRef")
}

fn normalize_variable_ref_value_for_pin(
    value: &mut flow_like_types::Value,
    pin_name: &str,
    lookup: &VariableRefLookup,
) {
    if !is_variable_ref_pin_name(pin_name) {
        return;
    }

    let flow_like_types::Value::String(raw) = value else {
        return;
    };

    if let Some(variable_id) = lookup.resolve(raw) {
        *raw = variable_id;
    }
}

fn find_input_pin<'a>(node: &'a Node, name: &str) -> Option<&'a Pin> {
    matching_input_pins(node, name).first().copied()
}

/// All input pins matching `name`, deterministically ordered: populated pins (connected or
/// holding a default — the ones lowering emits args for) first, then empty ones, each sorted
/// by pin index. `node.pins` is a HashMap, so an unsorted `.find()` picks an arbitrary pin
/// among same-named multi-pins and corrupts them nondeterministically.
fn matching_input_pins<'a>(node: &'a Node, name: &str) -> Vec<&'a Pin> {
    let mut matching: Vec<&Pin> = node
        .pins
        .values()
        .filter(|p| p.pin_type == PinType::Input && node_pin_name_matches(p, name))
        .collect();
    matching.sort_by_key(|p| {
        let populated = !p.depends_on.is_empty() || p.default_value.is_some();
        (!populated, p.index, p.id.clone())
    });
    matching
}

fn find_output_pin<'a>(node: &'a Node, name: &str) -> Option<&'a Pin> {
    let mut matching: Vec<&Pin> = node
        .pins
        .values()
        .filter(|p| p.pin_type == PinType::Output && node_pin_name_matches(p, name))
        .collect();
    matching.sort_by_key(|p| (p.index, p.id.clone()));
    matching.first().copied()
}

/// Read a pin's configured literal default as a JSON string value.
fn node_pin_literal_string(node: &Node, pin_name: &str) -> Option<String> {
    let pin = find_input_pin(node, pin_name)?;
    let bytes = pin.default_value.as_deref()?;
    match flow_like_types::json::from_slice::<flow_like_types::Value>(bytes).ok()? {
        flow_like_types::Value::String(value) => Some(value),
        _ => None,
    }
}

pub(crate) fn pin_name_matches(raw: &str, requested: &str) -> bool {
    raw == requested || to_camel_case(raw) == requested
}

fn node_pin_name_matches(pin: &Pin, requested: &str) -> bool {
    pin_name_matches(&pin.name, requested) || pin_name_matches(&pin.friendly_name, requested)
}

fn metadata_pin_name_matches(pin: &PinMetadata, requested: &str) -> bool {
    pin_name_matches(&pin.name, requested) || pin_name_matches(&pin.friendly_name, requested)
}

fn call_matches_node(call: &Call, node: &Node) -> bool {
    if !call.node_type.trim().is_empty() {
        return call.node_type == node.name;
    }

    pin_name_matches(&node.name, &call.display)
        || pin_name_matches(&node.friendly_name, &call.display)
}

fn is_exec_pin(pin: &Pin) -> bool {
    pin.data_type == VariableType::Execution
}

fn default_node_output_pin(node: &Node) -> Option<String> {
    let mut outputs: Vec<&Pin> = node
        .pins
        .values()
        .filter(|p| p.pin_type == PinType::Output && !is_exec_pin(p))
        .collect();
    outputs.sort_by_key(|p| p.index);
    match outputs.as_slice() {
        [pin] => Some(pin.name.clone()),
        many => many
            .iter()
            .find(|p| matches!(p.name.as_str(), "result" | "value" | "output" | "out"))
            .map(|p| p.name.clone()),
    }
}

fn exec_input_pin(node: &Node) -> Option<String> {
    node.pins
        .values()
        .filter(|p| p.pin_type == PinType::Input && is_exec_pin(p))
        .min_by_key(|p| p.index)
        .map(|p| p.name.clone())
}

fn exec_output_pin(node: &Node) -> Option<String> {
    let mut candidates: Vec<ExecPinCandidate> = node
        .pins
        .values()
        .filter(|p| p.pin_type == PinType::Output && is_exec_pin(p))
        .map(|p| ExecPinCandidate {
            name: p.name.clone(),
            friendly_name: p.friendly_name.clone(),
            index: p.index,
        })
        .collect();
    candidates.sort_by_key(|p| p.index);
    default_exec_output_by_policy(&node.name, &candidates)
}

fn metadata_input_pin<'a>(meta: &'a NodeMetadata, name: &str) -> Option<&'a PinMetadata> {
    metadata_input_pin_at(meta, name, 0)
}

/// The `occurrence`-th input pin matching `name` — multi-pins (several inputs sharing one name,
/// e.g. `bool_or`'s two `boolean` pins) pair positionally with same-named call arguments.
fn metadata_input_pin_at<'a>(
    meta: &'a NodeMetadata,
    name: &str,
    occurrence: usize,
) -> Option<&'a PinMetadata> {
    meta.inputs
        .iter()
        .filter(|p| p.data_type != "Execution" && metadata_pin_name_matches(p, name))
        .nth(occurrence)
}

/// Node types whose `on_update` mints one input pin per template placeholder, paired with the
/// config pin that carries the template. Reconcile plans NEW nodes against static catalog metadata
/// that predates those pins, so it predicts them from the config value here. Extend this list as
/// more placeholder-driven nodes appear (the prediction machinery is otherwise node-agnostic).
fn dynamic_placeholder_config_pin(node_type: &str) -> Option<&'static str> {
    match node_type {
        "string_format" => Some("format_string"),
        _ => None,
    }
}

/// Placeholder tokens in a template string, matching `string_format`'s `\{([a-zA-Z0-9_]+)\}`.
fn template_placeholders(template: &str) -> Vec<String> {
    let bytes = template.as_bytes();
    let mut names = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{' {
            let start = i + 1;
            let mut end = start;
            while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
                end += 1;
            }
            if end > start && bytes.get(end) == Some(&b'}') {
                names.push(template[start..end].to_string());
                i = end + 1;
                continue;
            }
        }
        i += 1;
    }
    names
}

/// The literal template string driving a placeholder node's dynamic pins: the value of the config
/// arg on this call if it is a string literal, else the existing node's stored config default.
fn placeholder_template_value(
    meta: &NodeMetadata,
    call: &Call,
    entity: &NodeEntity,
    existing: &Board,
    config_pin: &str,
) -> Option<String> {
    for arg in &call.args {
        if metadata_input_pin(meta, &arg.name).is_some_and(|pin| pin.name == config_pin) {
            return match &arg.value {
                Expr::Literal(Literal::String(template)) => Some(template.clone()),
                _ => None,
            };
        }
    }

    let NodeEntity::Existing(node_id) = entity else {
        return None;
    };
    let node = find_board_node(existing, node_id)?;
    let bytes = find_input_pin(node, config_pin)?.default_value.as_deref()?;
    flow_like_types::json::from_slice::<flow_like_types::Value>(bytes)
        .ok()?
        .as_str()
        .map(ToOwned::to_owned)
}

/// Permissive Generic input pin, mirroring the `VariableType::Generic` pins a placeholder node's
/// `on_update` adds. Generic short-circuits `metadata_pins_are_compatible`, so it accepts any wire.
fn generic_input_pin_metadata(name: &str) -> PinMetadata {
    PinMetadata {
        name: name.to_string(),
        friendly_name: name.to_string(),
        description: String::new(),
        data_type: "Generic".to_string(),
        value_type: "Normal".to_string(),
        default_value: None,
        schema: None,
        is_generic: true,
        valid_values: None,
        enforce_schema: false,
    }
}

/// Predict the dynamic input pin an `on_update` will add for `arg`, when it names a template
/// placeholder of a placeholder-driven node. Returns `None` for genuinely unknown pins so real
/// typos still surface as diagnostics.
fn synthesize_dynamic_input_pin(
    meta: &NodeMetadata,
    call: &Call,
    arg: &Arg,
    entity: &NodeEntity,
    existing: &Board,
) -> Option<PinMetadata> {
    let config_pin = dynamic_placeholder_config_pin(&meta.name)?;
    let template = placeholder_template_value(meta, call, entity, existing, config_pin)?;
    template_placeholders(&template)
        .iter()
        .any(|token| token == &arg.name || to_camel_case(token) == arg.name)
        .then(|| generic_input_pin_metadata(&arg.name))
}

/// Whether `arg` targets a dynamic input pin the node's `on_update` will mint (one not yet live on
/// the board node). Used by the config-edit path to DEFER a literal to apply — which creates the pin
/// via `on_update`, then applies the write — instead of reporting a missing pin. Prefers the
/// enricher (runs `on_update`); falls back to `synthesize_dynamic_input_pin` (string_format family).
fn arg_targets_predicted_dynamic_pin(
    node: &Node,
    node_id: &str,
    call: &Call,
    arg: &Arg,
    existing: &Board,
    enricher: Option<&MetadataEnricher>,
) -> bool {
    if find_input_pin(node, &arg.name).is_some() {
        return false;
    }
    let base = node_to_metadata(node);
    if let Some(enricher) = enricher {
        let literal_args: Vec<(String, flow_like_types::Value)> = call
            .args
            .iter()
            .filter_map(|a| literal_expr_to_value(&a.value).map(|value| (a.name.clone(), value)))
            .collect();
        if let Some(enriched) = enricher(&base, &literal_args, existing) {
            return enriched
                .inputs
                .iter()
                .any(|pin| metadata_pin_name_matches(pin, &arg.name));
        }
    }
    let entity = NodeEntity::Existing(node_id.to_string());
    synthesize_dynamic_input_pin(&base, call, arg, &entity, existing).is_some()
}

/// The node type + response pin an event/tool-entry `return` maps to (mirrors the `lower.rs`
/// `events_generic_return_result` sugar).
const EVENT_RETURN_RESULT_TYPE: &str = "events_generic_return_result";
const EVENT_RESPONSE_PIN: &str = "response";

/// Synthetic argument names the FlowScript decompiler emits for a node's function references
/// (`tools:` for agent tool-registration nodes, `fnRefs:` for generic references — see
/// `lower.rs::fn_ref_arg`). They are NOT board input pins, so reconcile must not treat them as
/// missing pins.
const SYNTHETIC_FN_REF_ARGS: &[&str] = &["tools", "fnRefs"];

/// A `tools:`/`fnRefs:` array carrying a node's function references, rather than a real pin
/// argument. Recognized so reconcile skips it (round-tripping to a no-op) instead of reporting a
/// missing pin — which otherwise surfaces to the user as a spurious "FlowScript apply blocked".
fn is_synthetic_fn_ref_arg(arg: &Arg) -> bool {
    SYNTHETIC_FN_REF_ARGS.contains(&arg.name.as_str()) && matches!(arg.value, Expr::Array(_))
}

/// Extract the referenced target names from a synthetic `tools:`/`fnRefs:` array argument
/// (`[fetchPage, …]`). Only bare references are recognized; the applier resolves each name to a
/// concrete node id.
fn synthetic_fn_ref_targets(arg: &Arg) -> Vec<String> {
    let Expr::Array(items) = &arg.value else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| match item {
            Expr::Ref(name) => Some(name.clone()),
            _ => None,
        })
        .collect()
}

fn metadata_output_pin<'a>(meta: &'a NodeMetadata, name: &str) -> Option<&'a PinMetadata> {
    meta.outputs
        .iter()
        .find(|p| p.data_type != "Execution" && metadata_pin_name_matches(p, name))
}

fn metadata_pins_are_compatible(input: &PinMetadata, output: &PinMetadata) -> bool {
    let data_type_ok = input.is_generic
        || output.is_generic
        || input.data_type == "Generic"
        || output.data_type == "Generic"
        || input.data_type == output.data_type;
    if !data_type_ok {
        return false;
    }

    input.value_type == output.value_type
        || input.value_type == "Normal"
        || output.value_type == "Normal"
}

fn default_metadata_output_pin(meta: &NodeMetadata) -> Option<String> {
    let outputs: Vec<&PinMetadata> = meta
        .outputs
        .iter()
        .filter(|p| p.data_type != "Execution")
        .collect();
    match outputs.as_slice() {
        [pin] => Some(pin.name.clone()),
        many => many
            .iter()
            .find(|p| matches!(p.name.as_str(), "result" | "value" | "output" | "out"))
            .map(|p| p.name.clone()),
    }
}

fn metadata_exec_input_pin(meta: &NodeMetadata) -> Option<String> {
    meta.inputs
        .iter()
        .find(|p| p.data_type == "Execution")
        .map(|p| p.name.clone())
}

fn metadata_exec_output_pin(meta: &NodeMetadata) -> Option<String> {
    let candidates: Vec<ExecPinCandidate> = meta
        .outputs
        .iter()
        .filter(|p| p.data_type == "Execution")
        .enumerate()
        .map(|(index, p)| ExecPinCandidate {
            name: p.name.clone(),
            friendly_name: p.friendly_name.clone(),
            index: index as u16,
        })
        .collect();
    default_exec_output_by_policy(&meta.name, &candidates)
}

#[derive(Debug, Clone)]
struct ExecPinCandidate {
    name: String,
    friendly_name: String,
    index: u16,
}

type ExecOutputSelector = fn(&[ExecPinCandidate]) -> Option<String>;

// Default sequential FlowScript execution is the reverse of lower.rs's statement-order rendering,
// but for multi-output nodes it is a semantic policy, not a pin-order guess.
// Nodes with more than one execution output must be listed here. Package/custom nodes should
// eventually surface this as catalog metadata; until then this registry is the authoritative
// callback map for "continue after this call" semantics.
const EXEC_OUTPUT_POLICIES: &[(&str, ExecOutputSelector)] = &[
    ("http_fetch", select_exec_success),
    ("streaming_http_fetch", select_exec_success),
];

fn default_exec_output_by_policy(
    node_type: &str,
    candidates: &[ExecPinCandidate],
) -> Option<String> {
    match candidates {
        [] => None,
        [single] => Some(single.name.clone()),
        many => {
            if let Some((_, selector)) = EXEC_OUTPUT_POLICIES
                .iter()
                .find(|(mapped_node_type, _)| *mapped_node_type == node_type)
            {
                return selector(many);
            }
            select_exec_done(many)
        }
    }
}

fn select_exec_success(candidates: &[ExecPinCandidate]) -> Option<String> {
    select_named_exec_pin(candidates, &["exec_success"])
}

fn select_exec_done(candidates: &[ExecPinCandidate]) -> Option<String> {
    select_named_exec_pin(candidates, &["exec_done", "done"])
}

fn select_named_exec_pin(candidates: &[ExecPinCandidate], names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| {
        candidates
            .iter()
            .find(|pin| pin.name == *name || pin.friendly_name == *name)
            .map(|pin| pin.name.clone())
    })
}

fn is_streaming_data_output_pin_name(name: &str) -> bool {
    matches!(
        name,
        "chunk" | "token" | "delta" | "stream_chunk" | "streamed_chunk"
    )
}

#[derive(Debug, Clone)]
enum NodeEntity {
    Existing(String),
    New {
        ref_id: String,
        meta: NodeMetadata,
    },
    Layer {
        ref_id: String,
        pins: Vec<LayerPinMetadata>,
    },
}

impl NodeEntity {
    fn node_ref(&self) -> String {
        match self {
            Self::Existing(id) => id.clone(),
            Self::New { ref_id, .. } => ref_id.clone(),
            Self::Layer { ref_id, .. } => ref_id.clone(),
        }
    }
}

#[derive(Debug, Clone)]
struct LayerPinMetadata {
    name: String,
    friendly_name: String,
    data_type: String,
    value_type: String,
    pin_type: String,
}

#[derive(Debug, Clone)]
struct ExecCursor {
    entity: NodeEntity,
    output_pin: Option<String>,
}

impl ExecCursor {
    fn new(entity: NodeEntity) -> Self {
        Self {
            entity,
            output_pin: None,
        }
    }

    fn with_output(entity: NodeEntity, output_pin: Option<String>) -> Self {
        Self { entity, output_pin }
    }
}

#[derive(Debug, Clone)]
struct PlannedStmt {
    entity: NodeEntity,
    next_exec_pin: Option<String>,
    input_sources: Vec<ValueSource>,
    /// Additional execution tails the NEXT statement must also wire from — the final cursors of
    /// branch arms. Execution inputs are legal fan-in points, so `if (x) { a() } b()` wires `b`
    /// from BOTH the untaken `false` pin and `a`'s exec output.
    extra_exec_tails: Vec<ExecCursor>,
    /// The statement's own entity does not continue the chain (a fully-armed branch): only
    /// `extra_exec_tails` carry execution forward.
    suppress_self_continuation: bool,
}

impl PlannedStmt {
    fn new(entity: NodeEntity) -> Self {
        Self {
            entity,
            next_exec_pin: None,
            input_sources: Vec::new(),
            extra_exec_tails: Vec::new(),
            suppress_self_continuation: false,
        }
    }

    fn with_next_exec_pin(entity: NodeEntity, next_exec_pin: Option<String>) -> Self {
        Self {
            next_exec_pin,
            ..Self::new(entity)
        }
    }

    fn with_input_sources(entity: NodeEntity, input_sources: Vec<ValueSource>) -> Self {
        Self {
            input_sources,
            ..Self::new(entity)
        }
    }

    fn next_cursor(&self) -> ExecCursor {
        ExecCursor::with_output(self.entity.clone(), self.next_exec_pin.clone())
    }
}

#[derive(Debug, Clone)]
struct ValueSource {
    node: NodeEntity,
    output_pin: Option<String>,
}

#[derive(Debug, Clone)]
enum SymbolValue {
    Source(ValueSource),
    Literal(flow_like_types::Value),
    VariableRef { variable_id: String },
}

struct CatalogIndex {
    by_display: HashMap<String, Vec<NodeMetadata>>,
    by_display_lower: HashMap<String, Vec<NodeMetadata>>,
    by_type: HashMap<String, NodeMetadata>,
}

impl CatalogIndex {
    fn new(catalog: &[NodeMetadata]) -> Self {
        let mut by_display: HashMap<String, Vec<NodeMetadata>> = HashMap::new();
        let mut by_display_lower: HashMap<String, Vec<NodeMetadata>> = HashMap::new();
        let mut by_type = HashMap::new();
        for meta in catalog {
            let display = to_camel_case(&meta.name);
            by_display
                .entry(display.clone())
                .or_default()
                .push(meta.clone());
            by_display_lower
                .entry(display.to_lowercase())
                .or_default()
                .push(meta.clone());
            by_type.insert(meta.name.clone(), meta.clone());
        }
        Self {
            by_display,
            by_display_lower,
            by_type,
        }
    }

    fn resolve_call(&self, call: &Call) -> Result<NodeMetadata, String> {
        if !call.node_type.trim().is_empty()
            && let Some(meta) = self.by_type.get(&call.node_type)
        {
            return Ok(meta.clone());
        }

        self.resolve_display(&call.display)
    }

    fn resolve_display(&self, display: &str) -> Result<NodeMetadata, String> {
        if display.trim().is_empty() {
            return Err("empty call display cannot be resolved to a catalog node".to_string());
        }

        if let Some(matches) = self.by_display.get(display) {
            return one_catalog_match(display, matches);
        }

        if let Some(matches) = self.by_display_lower.get(&display.to_lowercase()) {
            return one_catalog_match(display, matches);
        }

        Err(format!(
            "FlowScript call `{display}` does not match a catalog declaration; call `get_declarations` and use the exact function name"
        ))
    }
}

fn one_catalog_match(display: &str, matches: &[NodeMetadata]) -> Result<NodeMetadata, String> {
    match matches {
        [single] => Ok(single.clone()),
        [] => Err(format!(
            "FlowScript call `{display}` did not match the catalog"
        )),
        many => {
            // Board-derived catalogs carry one entry per node INSTANCE — identical node types
            // are one logical declaration, not an ambiguity.
            let first = &many[0];
            if many.iter().all(|m| m.name == first.name) {
                return Ok(first.clone());
            }
            let node_types: Vec<&str> = many.iter().map(|m| m.name.as_str()).collect();
            Err(format!(
                "FlowScript call `{display}` is ambiguous; matched {}",
                node_types.join(", ")
            ))
        }
    }
}

struct BoardIndex<'a> {
    pin_owner: HashMap<&'a str, (&'a Node, &'a Pin)>,
}

impl<'a> BoardIndex<'a> {
    fn new(board: &'a Board) -> Self {
        let mut pin_owner = HashMap::new();
        for node in board.nodes.values() {
            for pin in node.pins.values() {
                pin_owner.insert(pin.id.as_str(), (node, pin));
            }
        }
        for layer in board.layers.values() {
            for node in layer.nodes.values() {
                for pin in node.pins.values() {
                    pin_owner.insert(pin.id.as_str(), (node, pin));
                }
            }
        }
        Self { pin_owner }
    }

    fn exec_incoming_edges(&self, node: &Node, input_pin_name: &str) -> Vec<(String, String)> {
        let Some(input) = find_input_pin(node, input_pin_name) else {
            return Vec::new();
        };
        if !is_exec_pin(input) {
            return Vec::new();
        }
        input
            .depends_on
            .iter()
            .filter_map(|pin_id| {
                self.pin_owner
                    .get(pin_id.as_str())
                    .map(|(source_node, source_pin)| {
                        (source_node.id.clone(), source_pin.name.clone())
                    })
            })
            .collect()
    }

    fn data_source_for_input(&self, node: &Node, input_pin_name: &str) -> Option<ValueSource> {
        let input = find_input_pin(node, input_pin_name)?;
        if is_exec_pin(input) {
            return None;
        }
        let mut source_pin_id = input.depends_on.iter().next()?;
        // Reroutes are pure wire-bends that the renderer collapses; trace through them so the
        // reuse path sees the real origin node — the one the FlowScript text actually shows.
        // Otherwise every inline chain behind a reroute is re-created on reconcile.
        for _ in 0..64 {
            let (source_node, source_pin) = self.pin_owner.get(source_pin_id.as_str())?;
            if source_node.name != "reroute" {
                return Some(ValueSource {
                    node: NodeEntity::Existing(source_node.id.clone()),
                    output_pin: Some(source_pin.name.clone()),
                });
            }
            let route_in = find_input_pin(source_node, "route_in")?;
            source_pin_id = route_in.depends_on.iter().next()?;
        }
        None
    }
}

struct StructuralPlanner<'a> {
    existing: &'a Board,
    board_index: BoardIndex<'a>,
    catalog: CatalogIndex,
    result: ReconcileResult,
    add_commands: Vec<BoardCommand>,
    disconnect_commands: Vec<BoardCommand>,
    connect_commands: Vec<BoardCommand>,
    update_commands: Vec<BoardCommand>,
    symbols: Vec<HashMap<String, SymbolValue>>,
    variable_refs: VariableRefLookup,
    function_return_targets: Vec<(NodeEntity, Vec<String>)>,
    unresolved_variable_refs: HashSet<String>,
    /// Newly added impure nodes: (ref_id, execution input pin, friendly name). Checked at the end
    /// for a missing incoming execution edge.
    new_impure_nodes: Vec<(String, String, String)>,
    /// Ref ids exempt from the dangling-execution check (a function body's first node, which has no
    /// execution entry to wire from yet).
    exec_check_exempt: HashSet<String>,
    next_ref: usize,
    next_position: usize,
    /// Impure calls planned in expression/argument position (e.g. `value: fakerFullName()`).
    /// They only receive data wiring where they are resolved; `plan_block` drains this queue and
    /// splices them into the exec chain ahead of the consuming statement (innermost call first —
    /// the natural push order of the expression recursion). Without this their exec pins dangle
    /// and the nodes never run.
    pending_exec_splices: Vec<NodeEntity>,
    /// `SetNodeFunctionRefs` commands synthesized from `tools:`/`fnRefs:` arguments on newly added
    /// nodes. Held separately so they emit after the add/connect commands (the applier resolves the
    /// referenced targets — function layers, events — once those nodes exist).
    fn_ref_commands: Vec<BoardCommand>,
    /// Declared FlowScript functions by camelCase name, pre-created before events/bodies are
    /// planned so `functionName(...)` call sites resolve to `control_call_function` nodes.
    planned_functions: HashMap<String, PlannedFunction>,
    /// Optional hook to materialize a node's dynamic (`on_update`-generated) pins for a call, so a
    /// literal/connection targeting one resolves against a real pin instead of the predicted
    /// `synthesize_dynamic_input_pin` fallback. `None` for the pure static-catalog paths.
    enricher: Option<&'a MetadataEnricher>,
}

impl<'a> StructuralPlanner<'a> {
    fn new(
        existing: &'a Board,
        catalog: &[NodeMetadata],
        enricher: Option<&'a MetadataEnricher>,
    ) -> Self {
        Self {
            existing,
            board_index: BoardIndex::new(existing),
            catalog: CatalogIndex::new(catalog),
            result: ReconcileResult::default(),
            add_commands: Vec::new(),
            disconnect_commands: Vec::new(),
            connect_commands: Vec::new(),
            update_commands: Vec::new(),
            symbols: Vec::new(),
            variable_refs: VariableRefLookup::from_board(existing),
            function_return_targets: Vec::new(),
            unresolved_variable_refs: HashSet::new(),
            new_impure_nodes: Vec::new(),
            exec_check_exempt: HashSet::new(),
            next_ref: 0,
            next_position: 0,
            pending_exec_splices: Vec::new(),
            fn_ref_commands: Vec::new(),
            planned_functions: HashMap::new(),
            enricher,
        }
    }

    /// Enrich a resolved node's metadata with the dynamic pins its `on_update` would create for this
    /// call's literal arguments, so the reconciler can resolve those pins. No-op without an enricher
    /// (the default for tests and the non-enriched entry points).
    fn enrich_meta(&self, meta: NodeMetadata, call: &Call) -> NodeMetadata {
        let Some(enricher) = self.enricher else {
            return meta;
        };
        let literal_args: Vec<(String, flow_like_types::Value)> = call
            .args
            .iter()
            .filter_map(|arg| {
                literal_expr_to_value(&arg.value).map(|value| (arg.name.clone(), value))
            })
            .collect();
        enricher(&meta, &literal_args, self.existing).unwrap_or(meta)
    }

    fn plan(mut self, ast: &BoardAst) -> ReconcileResult {
        self.push_scope();
        self.seed_top_level_variables(ast);
        // Function layers are created (and their impurity decided) up front so call sites in
        // events and other functions resolve regardless of declaration order.
        self.prepare_functions(ast);
        for event in &ast.events {
            self.plan_event(event, None);
        }
        for func in &ast.functions {
            self.plan_function_body(func);
        }
        self.pop_scope();

        self.check_dangling_impure_execution();

        self.result.commands.extend(self.add_commands);
        // Literal/config updates can change node shape (for example format/schema/template
        // nodes whose on_update adds dynamic pins), so apply them before resolving connections.
        self.result.commands.extend(self.update_commands);
        self.result.commands.extend(self.disconnect_commands);
        self.result.commands.extend(self.connect_commands);
        // Function references resolve against the nodes/layers added above, so emit last.
        self.result.commands.extend(self.fn_ref_commands);
        self.result
    }

    fn prepare_functions(&mut self, ast: &BoardAst) {
        for func in &ast.functions {
            let mut seen = HashSet::new();
            seen.insert(func.name.clone());
            let impure = self.function_body_is_impure(ast, &func.body, &mut seen);
            let entity = self.function_layer_entity(func, impure);
            self.planned_functions.insert(
                func.name.clone(),
                PlannedFunction {
                    entity,
                    impure,
                    params: func.params.clone(),
                    returns: func.returns.clone(),
                },
            );
        }
    }

    /// Static prescan: does this function body contain anything that needs an execution chain
    /// (impure catalog calls, control flow, board-variable writes, or calls to other impure
    /// functions)? Decides whether the layer gets `exec_in`/`exec_out` boundary pins — a pure
    /// function must NOT carry them (the runtime would then look for an entry node), and an
    /// impure one must (or its body never runs when called).
    fn function_body_is_impure(
        &self,
        ast: &BoardAst,
        block: &Block,
        seen: &mut HashSet<String>,
    ) -> bool {
        block.stmts.iter().any(|stmt| match stmt {
            Stmt::Branch { .. } | Stmt::Loop { .. } => true,
            Stmt::Let { call, .. } | Stmt::Call { call, .. } => {
                self.call_is_impure(ast, call, seen)
            }
            Stmt::Assign { target, value, .. } => {
                self.assignment_targets_board_variable(ast, target)
                    || self.expr_contains_impure_call(ast, value, seen)
            }
            // `base.path = value` plans as an Assign to `base` (struct_set accumulator) — a
            // board-variable base ends in an impure variable_set node just like a plain Assign.
            Stmt::FieldAssign { base, value, .. } => {
                self.assignment_targets_board_variable(ast, base)
                    || self.expr_contains_impure_call(ast, value, seen)
            }
            Stmt::LocalAlias { value, .. } => self.expr_contains_impure_call(ast, value, seen),
            Stmt::Return { values, .. } => values
                .iter()
                .any(|value| self.expr_contains_impure_call(ast, value, seen)),
            Stmt::Handler(_) | Stmt::Local(_) | Stmt::Comment(_) => false,
        })
    }

    fn call_is_impure(&self, ast: &BoardAst, call: &Call, seen: &mut HashSet<String>) -> bool {
        // Impure calls hidden inside the ARGUMENTS make the enclosing statement impure no matter
        // what the callee is — they get exec-spliced ahead of it at plan time.
        let args_impure = call
            .args
            .iter()
            .any(|arg| self.expr_contains_impure_call(ast, &arg.value, seen));
        if let Some(func) = ast
            .functions
            .iter()
            .find(|func| func.name == call.display)
        {
            let body_impure = seen.insert(func.name.clone())
                && self.function_body_is_impure(ast, &func.body, seen);
            return body_impure || args_impure;
        }
        let impure_by_meta = self
            .catalog
            .resolve_call(call)
            .ok()
            .is_some_and(|meta| metadata_exec_input_pin(&meta).is_some());
        impure_by_meta || args_impure
    }

    fn expr_contains_impure_call(
        &self,
        ast: &BoardAst,
        expr: &Expr,
        seen: &mut HashSet<String>,
    ) -> bool {
        match expr {
            Expr::Call(call) => self.call_is_impure(ast, call, seen),
            Expr::Field { base, .. } | Expr::Member { base, .. } => {
                self.expr_contains_impure_call(ast, base, seen)
            }
            Expr::Object(fields) => fields
                .iter()
                .any(|field| self.expr_contains_impure_call(ast, &field.value, seen)),
            Expr::Array(items) => items
                .iter()
                .any(|item| self.expr_contains_impure_call(ast, item, seen)),
            Expr::Index { base, index } => {
                self.expr_contains_impure_call(ast, base, seen)
                    || self.expr_contains_impure_call(ast, index, seen)
            }
            Expr::Ternary {
                cond,
                then,
                otherwise,
            } => {
                self.expr_contains_impure_call(ast, cond, seen)
                    || self.expr_contains_impure_call(ast, then, seen)
                    || self.expr_contains_impure_call(ast, otherwise, seen)
            }
            Expr::Binary { lhs, rhs, .. } => {
                self.expr_contains_impure_call(ast, lhs, seen)
                    || self.expr_contains_impure_call(ast, rhs, seen)
            }
            Expr::Ref(_) | Expr::Literal(_) => false,
        }
    }

    fn assignment_targets_board_variable(&self, ast: &BoardAst, target: &str) -> bool {
        ast.variables.iter().any(|var| var.name == target)
            || self.variable_refs.resolve(target).is_some()
    }

    fn plan_event(&mut self, event: &EventBlock, target_layer: Option<String>) {
        let entry = match &event.anchor {
            Some(anchor) => {
                let entry = find_board_node(self.existing, anchor)
                    .map(|_| NodeEntity::Existing(anchor.clone()));
                if entry.is_none() {
                    self.result.diagnostics.push(format!(
                        "event `{}` anchors to `{anchor}`, which no longer exists on the board; its body was planned without an execution entry",
                        event.name
                    ));
                }
                entry
            }
            None => {
                self.add_entry_node(&event.name, target_layer.clone(), !event.params.is_empty())
            }
        };

        self.push_scope();
        if let Some(entry) = &entry {
            self.seed_params_from_entity(&event.params, entry);
        }
        self.plan_block(&event.body, entry.map(ExecCursor::new), target_layer);
        self.pop_scope();
    }

    fn plan_function_body(&mut self, func: &FnDecl) {
        let Some(planned) = self.planned_functions.get(&func.name).cloned() else {
            return;
        };
        let layer = planned.entity.clone();
        let target_layer = Some(layer.node_ref());
        self.push_scope();
        self.seed_function_params(&func.params, &layer);
        self.function_return_targets.push((
            layer.clone(),
            func.returns
                .iter()
                .map(|param| param.name.clone())
                .collect(),
        ));
        let entry = if planned.impure {
            self.function_entry_cursor(&layer, &func.name)
        } else {
            None
        };
        let final_cursors = self.plan_block(&func.body, entry, target_layer);
        if planned.impure {
            for cursor in final_cursors {
                self.wire_function_exit(&layer, cursor);
            }
        }
        self.function_return_targets.pop();
        self.pop_scope();
    }

    /// Execution entry for an impure function body: the layer's `exec_in` boundary pin. The
    /// runtime `control_call_function` node finds the body's entry node by following exactly this
    /// edge, so without it the body would never run.
    fn function_entry_cursor(&mut self, layer: &NodeEntity, name: &str) -> Option<ExecCursor> {
        match layer {
            NodeEntity::Layer { .. } => Some(ExecCursor::with_output(
                layer.clone(),
                Some(FUNCTION_EXEC_IN.to_string()),
            )),
            NodeEntity::Existing(id) => {
                let exec_in = self.existing.layers.get(id).and_then(|existing_layer| {
                    existing_layer
                        .pins
                        .values()
                        .find(|pin| {
                            pin.pin_type == PinType::Input
                                && pin.data_type == VariableType::Execution
                        })
                        .map(|pin| pin.name.clone())
                });
                match exec_in {
                    Some(pin) => Some(ExecCursor::with_output(layer.clone(), Some(pin))),
                    None => {
                        self.result.diagnostics.push(format!(
                            "function `{name}` has impure statements but its existing layer has no execution boundary pins; its body will not run when called. Recreate the function (delete and re-add the `function` declaration) to migrate it"
                        ));
                        None
                    }
                }
            }
            NodeEntity::New { .. } => None,
        }
    }

    /// Close an impure function body: wire a final statement's execution output to the layer's
    /// `exec_out` boundary pin so the graph reads (and renders) as a complete chain. Called once
    /// per final cursor — branch tails legally fan into the boundary pin.
    fn wire_function_exit(&mut self, layer: &NodeEntity, cursor: ExecCursor) {
        if matches!(&cursor.entity, NodeEntity::Existing(_))
            && matches!(layer, NodeEntity::Existing(_))
        {
            return;
        }
        let Some(from_pin) = cursor
            .output_pin
            .clone()
            .or_else(|| self.entity_exec_output_pin(&cursor.entity))
        else {
            return;
        };
        // The body's first statement wires FROM the layer's exec_in; if the body is empty the
        // cursor still points at the layer itself — nothing to close then.
        if cursor.entity.node_ref() == layer.node_ref() {
            return;
        }
        let to_pin = match layer {
            NodeEntity::Layer { .. } => Some(FUNCTION_EXEC_OUT.to_string()),
            NodeEntity::Existing(id) => self.existing.layers.get(id).and_then(|existing_layer| {
                existing_layer
                    .pins
                    .values()
                    .find(|pin| {
                        pin.pin_type == PinType::Output
                            && pin.data_type == VariableType::Execution
                    })
                    .map(|pin| pin.name.clone())
            }),
            NodeEntity::New { .. } => None,
        };
        let Some(to_pin) = to_pin else { return };
        self.connect_commands.push(BoardCommand::ConnectPins {
            from_node: cursor.entity.node_ref(),
            from_pin,
            to_node: layer.node_ref(),
            to_pin,
            summary: Some("Connect function body to its exec out".to_string()),
        });
    }

    /// Plan a statement block. Returns the final execution cursors — where the chain "ends" — so
    /// function bodies can close onto their layer's `exec_out` boundary pin. The cursor SET
    /// models fan-in: after `if (x) { a() }`, the next statement wires from both the untaken
    /// `false` pin and `a`'s exec output.
    fn plan_block(
        &mut self,
        block: &Block,
        entry: Option<ExecCursor>,
        target_layer: Option<String>,
    ) -> Vec<ExecCursor> {
        let mut previous_execs: Vec<ExecCursor> = entry.into_iter().collect();
        // The `(node, pin)` exec edge the current insertion streak branched off from — set
        // when a NEW node is first wired after an EXISTING one, and used to splice out only
        // that predecessor's old edge when the chain reaches the next existing node.
        let mut insertion_origin: Option<(String, String)> = None;
        let promoted_local_aliases = promoted_local_aliases(block);

        for stmt in &block.stmts {
            let planned = self.plan_stmt(
                stmt,
                target_layer.clone(),
                matches!(
                    stmt,
                    Stmt::LocalAlias { name, .. } if promoted_local_aliases.contains(name)
                ),
            );

            // Splice impure argument calls (data-wired during expression resolution) into the
            // exec chain ahead of the statement that consumes their outputs, innermost first,
            // so they actually execute at runtime.
            for splice in std::mem::take(&mut self.pending_exec_splices) {
                for previous in &previous_execs {
                    let connected_edge =
                        self.connect_exec(previous, &splice, insertion_origin.as_ref());
                    if let Some(edge) = connected_edge
                        && insertion_origin.is_none()
                        && matches!(previous.entity, NodeEntity::Existing(_))
                        && matches!(splice, NodeEntity::New { .. })
                    {
                        insertion_origin = Some(edge);
                    }
                }
                previous_execs = vec![ExecCursor::new(splice)];
            }

            let Some(current) = planned else {
                continue;
            };

            let accepts_exec = self.entity_exec_input_pin(&current.entity).is_some();
            let continues_exec = current.next_exec_pin.is_some()
                || (accepts_exec && !self.entity_exec_output_pins(&current.entity).is_empty());

            if accepts_exec {
                if previous_execs.is_empty() {
                    // No execution predecessor to wire from (e.g. a function body's first node):
                    // exempt it from the dangling-execution warning.
                    if let NodeEntity::New { ref_id, .. } = &current.entity {
                        self.exec_check_exempt.insert(ref_id.clone());
                    }
                } else {
                    // The streaming side-chain special case only exists for a single linear
                    // predecessor; fan-in sets (branch tails) never stream.
                    let single_previous = previous_execs.len() == 1;
                    let mut statement_is_side_chain = false;
                    for previous in previous_execs.clone() {
                        let preferred_output = self.preferred_exec_output_for_input_sources(
                            &previous.entity,
                            &current.input_sources,
                        );
                        // Only a streaming-preferred branch (on_stream/on_chunk/…) is a
                        // side-chain that must not advance the main exec cursor. A cursor that
                        // merely carries a non-default continuation pin (loop body entry, loop
                        // "done") IS the main chain.
                        let used_branch_output = single_previous
                            && preferred_output.is_some()
                            && self.entity_exec_output_pin(&previous.entity) != preferred_output;
                        let previous = match preferred_output {
                            Some(output_pin) => {
                                ExecCursor::with_output(previous.entity.clone(), Some(output_pin))
                            }
                            None => previous,
                        };
                        let connected_edge = self.connect_exec(
                            &previous,
                            &current.entity,
                            insertion_origin.as_ref(),
                        );
                        if let Some(edge) = connected_edge
                            && !used_branch_output
                            && insertion_origin.is_none()
                            && matches!(previous.entity, NodeEntity::Existing(_))
                            && matches!(current.entity, NodeEntity::New { .. })
                        {
                            insertion_origin = Some(edge);
                        }
                        if used_branch_output {
                            statement_is_side_chain = true;
                        }
                    }
                    if statement_is_side_chain {
                        continue;
                    }
                }
            }

            if matches!(current.entity, NodeEntity::Existing(_)) {
                insertion_origin = None;
            }

            let mut next_cursors = current.extra_exec_tails.clone();
            if continues_exec && !current.suppress_self_continuation {
                next_cursors.push(current.next_cursor());
            }
            if !next_cursors.is_empty() {
                previous_execs = next_cursors;
            } else if accepts_exec || current.suppress_self_continuation {
                previous_execs = Vec::new();
            }
        }
        previous_execs
    }

    fn plan_stmt(
        &mut self,
        stmt: &Stmt,
        target_layer: Option<String>,
        promote_local_alias: bool,
    ) -> Option<PlannedStmt> {
        match stmt {
            Stmt::Let { name, call, anchor } => {
                let planned_call =
                    self.plan_call_statement_with_sources(call, anchor.as_deref(), target_layer);
                let (entity, input_sources) = planned_call?;
                self.insert_symbol(
                    name.clone(),
                    SymbolValue::Source(ValueSource {
                        node: entity.clone(),
                        output_pin: None,
                    }),
                );
                Some(PlannedStmt::with_input_sources(entity, input_sources))
            }
            Stmt::Call { call, anchor } => {
                let (entity, input_sources) =
                    self.plan_call_statement_with_sources(call, anchor.as_deref(), target_layer)?;
                Some(PlannedStmt::with_input_sources(entity, input_sources))
            }
            Stmt::Assign {
                target,
                value,
                anchor,
            } => {
                if let Some(anchor) = anchor {
                    if let Some(variable_id) = self.variable_id_for_assignment_target(target) {
                        let entity = NodeEntity::Existing(anchor.clone());
                        self.plan_existing_variable_set_node(
                            &entity,
                            &variable_id,
                            value,
                            target_layer,
                        );
                        self.assign_symbol(
                            target.clone(),
                            SymbolValue::VariableRef {
                                variable_id: variable_id.clone(),
                            },
                        );
                        return Some(PlannedStmt::new(entity));
                    }

                    if let Some((call, output_pin)) = assigned_call_expr(value) {
                        let entity = self
                            .plan_call_statement(call, Some(anchor), target_layer)
                            .unwrap_or_else(|| NodeEntity::Existing(anchor.clone()));
                        let output_pin = output_pin
                            .and_then(|pin| self.resolve_entity_output_pin(&entity, Some(pin)));
                        self.insert_symbol(
                            target.clone(),
                            SymbolValue::Source(ValueSource {
                                node: entity.clone(),
                                output_pin,
                            }),
                        );
                        return Some(PlannedStmt::new(entity));
                    }

                    return Some(PlannedStmt::new(NodeEntity::Existing(anchor.clone())));
                }

                if let Some(variable_id) = self.variable_id_for_assignment_target(target) {
                    let entity =
                        self.add_variable_set_node(&variable_id, value, target_layer.clone());
                    self.assign_symbol(
                        target.clone(),
                        SymbolValue::VariableRef {
                            variable_id: variable_id.clone(),
                        },
                    );
                    return entity.map(PlannedStmt::new);
                }

                let Some(resolved) = self.resolve_expr(value, target_layer.clone()) else {
                    self.result.diagnostics.push(format!(
                        "assignment to `{target}` is not a literal or resolvable node output; skipped local alias"
                    ));
                    return None;
                };
                let entity = match (&resolved, value) {
                    (SymbolValue::Source(source), Expr::Call(_)) => Some(source.node.clone()),
                    (SymbolValue::Source(source), Expr::Field { base, .. })
                        if matches!(base.as_ref(), Expr::Call(_)) =>
                    {
                        Some(source.node.clone())
                    }
                    _ => None,
                };
                if let Some(entity) = &entity {
                    self.undefer_statement_call_splice(entity);
                }
                self.assign_symbol(target.clone(), resolved);
                entity.map(PlannedStmt::new)
            }
            Stmt::FieldAssign {
                base,
                path,
                value,
                anchor,
            } => {
                // Expand the `base.path = value` struct-field write to its `struct_set` accumulator
                // form and reuse the `Stmt::Assign`+call planning path: it wires `struct_in` from
                // `base`'s prior source, rebinds `base` to `struct_out`, and (via the non-anchored
                // path) drops the struct_set's own splice so it never self-connects.
                let call = field_assign_struct_set_call(base, path, value, anchor.as_deref());
                let assign = Stmt::Assign {
                    target: base.clone(),
                    value: Expr::Call(call),
                    anchor: anchor.clone(),
                };
                self.plan_stmt(&assign, target_layer, promote_local_alias)
            }
            Stmt::LocalAlias {
                name,
                value,
                anchor,
            } => {
                if let Some(anchor) = anchor {
                    if let Some((call, output_pin)) = assigned_call_expr(value) {
                        let entity = self
                            .plan_call_statement(call, Some(anchor), target_layer)
                            .unwrap_or_else(|| NodeEntity::Existing(anchor.clone()));
                        let output_pin = output_pin
                            .and_then(|pin| self.resolve_entity_output_pin(&entity, Some(pin)));
                        self.insert_symbol(
                            name.clone(),
                            SymbolValue::Source(ValueSource {
                                node: entity.clone(),
                                output_pin,
                            }),
                        );
                        return Some(PlannedStmt::new(entity));
                    }

                    return Some(PlannedStmt::new(NodeEntity::Existing(anchor.clone())));
                }

                if promote_local_alias {
                    let variable_id = self.create_local_variable(name, value, target_layer.clone());
                    self.insert_symbol(name.clone(), SymbolValue::VariableRef { variable_id });
                    return None;
                }

                let Some(resolved) = self.resolve_expr(value, target_layer.clone()) else {
                    self.result.diagnostics.push(format!(
                        "local alias `{name}` is not a literal or resolvable node output; skipped"
                    ));
                    return None;
                };
                let entity = match (&resolved, value) {
                    (SymbolValue::Source(source), Expr::Call(_)) => Some(source.node.clone()),
                    (SymbolValue::Source(source), Expr::Field { base, .. })
                        if matches!(base.as_ref(), Expr::Call(_)) =>
                    {
                        Some(source.node.clone())
                    }
                    _ => None,
                };
                if let Some(entity) = &entity {
                    self.undefer_statement_call_splice(entity);
                }
                self.insert_symbol(name.clone(), resolved);
                entity.map(PlannedStmt::new)
            }
            Stmt::Branch {
                bind,
                call,
                condition,
                arms,
                anchor,
            } => {
                let mut synthesized_branch = false;
                let entity = if let Some(bind) = bind
                    && is_placeholder_call(call)
                {
                    match self.lookup_symbol(bind) {
                        Some(SymbolValue::Source(source)) => Some(source.node),
                        _ => {
                            self.result.diagnostics.push(format!(
                                "branch binding `{bind}` does not resolve to a node output"
                            ));
                            None
                        }
                    }
                } else if anchor.is_none()
                    && bind.is_none()
                    && is_placeholder_call(call)
                    && condition.is_some()
                {
                    // New `if (cond) { ... } [else { ... }]` sugar: synthesize the
                    // `control_branch` node and wire the condition through the normal
                    // argument machinery. Arm labels True/False match its exec pins.
                    synthesized_branch = true;
                    let branch_call = Call {
                        node_type: "control_branch".to_string(),
                        display: "controlBranch".to_string(),
                        args: vec![Arg {
                            name: "condition".to_string(),
                            value: condition.clone().expect("condition checked above"),
                        }],
                        anchor: None,
                    };
                    self.plan_call_statement(&branch_call, None, target_layer.clone())
                } else {
                    let entity =
                        self.plan_call_statement(call, anchor.as_deref(), target_layer.clone());
                    if let (Some(bind), Some(entity)) = (bind, entity.as_ref()) {
                        self.insert_symbol(
                            bind.clone(),
                            SymbolValue::Source(ValueSource {
                                node: entity.clone(),
                                output_pin: None,
                            }),
                        );
                    }
                    entity
                };

                if anchor.is_none() && bind.is_none() && !synthesized_branch {
                    self.result.diagnostics.push(
                        "new FlowScript branch statements are not yet converted automatically; use the control_branch declaration or emit_commands for complex branch wiring".to_string(),
                    );
                }
                if let Some(cond) = condition {
                    // Anchored (existing) branches already have their condition wired on the
                    // board; a synthesized branch wired it through its own call arguments.
                    if !matches!(entity, Some(NodeEntity::Existing(_))) && !synthesized_branch {
                        let _ = self.resolve_expr(cond, target_layer.clone());
                    }
                }
                // The branch's own argument/condition splices must chain BEFORE the branch
                // node, not inside an arm — stash them across the nested blocks.
                let mut stashed_splices = std::mem::take(&mut self.pending_exec_splices);
                let mut arm_tails: Vec<ExecCursor> = Vec::new();
                for arm in arms {
                    self.push_scope();
                    // Each arm chains from ITS labelled exec output. Wiring every arm from the
                    // default/policy pin would make later arms silently steal earlier arms' (and
                    // the default continuation's) single-target exec edge.
                    let arm_cursor = entity.clone().and_then(|entity| {
                        match self.resolve_arm_exec_pin(&entity, &arm.label) {
                            Some(pin) => Some(ExecCursor::with_output(entity, Some(pin))),
                            None => {
                                if !arm.body.stmts.is_empty() {
                                    self.result.diagnostics.push(format!(
                                        "branch arm label `{}` does not match an execution output on `{}`; its body was not wired — use the exact exec pin name as the arm label",
                                        arm.label, call.display
                                    ));
                                }
                                None
                            }
                        }
                    });
                    // Each arm's final cursors carry execution to whatever follows the branch
                    // (exec inputs are fan-in points). An empty arm's tail is the labelled pin
                    // itself — the pass-through case.
                    arm_tails.extend(self.plan_block(&arm.body, arm_cursor, target_layer.clone()));
                    self.pop_scope();
                }
                stashed_splices.append(&mut self.pending_exec_splices);
                self.pending_exec_splices = stashed_splices;
                if bind.is_some() && is_placeholder_call(call) {
                    None
                } else {
                    entity.map(|entity| {
                        // The statement AFTER a branch continues from the arm tails plus the one
                        // exec output no arm claimed (e.g. `false` for a lone `if`). When every
                        // output is claimed, only the arm tails carry execution forward.
                        let arm_pins: HashSet<String> = arms
                            .iter()
                            .filter_map(|arm| self.resolve_arm_exec_pin(&entity, &arm.label))
                            .collect();
                        let remaining: Vec<String> = self
                            .entity_exec_output_pins(&entity)
                            .into_iter()
                            .filter(|pin| !arm_pins.contains(pin))
                            .collect();
                        let mut planned = PlannedStmt::new(entity);
                        planned.extra_exec_tails = arm_tails;
                        match remaining.as_slice() {
                            [single] => planned.next_exec_pin = Some(single.clone()),
                            [] => planned.suppress_self_continuation = true,
                            // Several unclaimed outputs (a bare multi-output call with one
                            // labelled arm): leave the default policy machinery to decide.
                            _ => {}
                        }
                        planned
                    })
                }
            }
            Stmt::Loop {
                bind,
                call,
                body,
                anchor,
                ..
            } => {
                let entity =
                    self.plan_call_statement(call, anchor.as_deref(), target_layer.clone());
                // The loop's own argument splices must chain BEFORE the loop node, not inside
                // its body — stash them across the nested plan_block.
                let mut stashed_splices = std::mem::take(&mut self.pending_exec_splices);
                self.push_scope();
                if let (Some(bind), Some(entity)) = (bind, entity.as_ref()) {
                    self.insert_symbol(
                        bind.clone(),
                        SymbolValue::Source(ValueSource {
                            node: entity.clone(),
                            output_pin: None,
                        }),
                    );
                }
                if let Some(entity) = entity.as_ref() {
                    let body_pin =
                        self.entity_exec_output_pin_named(entity, &["exec_out", "loop", "body"]);
                    self.plan_block(
                        body,
                        Some(ExecCursor::with_output(entity.clone(), body_pin)),
                        target_layer,
                    );
                }
                self.pop_scope();
                stashed_splices.append(&mut self.pending_exec_splices);
                self.pending_exec_splices = stashed_splices;
                entity.map(|entity| {
                    PlannedStmt::with_next_exec_pin(
                        entity.clone(),
                        self.entity_exec_output_pin_named(&entity, &["done", "exec_done"]),
                    )
                })
            }
            Stmt::Handler(event) => {
                // A nested handler is an independent entry point but lives (and counts) in the
                // enclosing scope's layer.
                self.plan_event(event, target_layer);
                None
            }
            Stmt::Return { values, anchor } => {
                self.plan_return(values, anchor.as_deref(), target_layer)
            }
            Stmt::Local(_) | Stmt::Comment(_) => None,
        }
    }

    fn function_layer_entity(&mut self, func: &FnDecl, impure: bool) -> NodeEntity {
        if let Some(anchor) = &func.anchor {
            return NodeEntity::Existing(anchor.clone());
        }

        let ref_id = format!("${}", self.next_ref);
        self.next_ref += 1;
        let pins = function_layer_pins(func, impure);
        let position = self.next_position();
        self.add_commands.push(BoardCommand::CreateLayer {
            name: func.name.clone(),
            ref_id: Some(ref_id.clone()),
            layer_type: Some("Function".to_string()),
            node_ids: Vec::new(),
            pins: Some(
                pins.iter()
                    .map(|pin| PlaceholderPinDef {
                        name: pin.name.clone(),
                        friendly_name: pin.friendly_name.clone(),
                        description: None,
                        pin_type: pin.pin_type.clone(),
                        data_type: pin.data_type.clone(),
                        value_type: Some(pin.value_type.clone()),
                    })
                    .collect(),
            ),
            position: Some(position),
            color: None,
            target_layer: None,
            summary: Some(format!("Create function {}", func.name)),
        });
        NodeEntity::Layer { ref_id, pins }
    }

    fn seed_function_params(&mut self, params: &[Param], layer: &NodeEntity) {
        for param in params {
            self.insert_symbol(
                param.name.clone(),
                SymbolValue::Source(ValueSource {
                    node: layer.clone(),
                    output_pin: Some(param.name.clone()),
                }),
            );
        }
    }

    fn plan_return(
        &mut self,
        values: &[Expr],
        anchor: Option<&str>,
        target_layer: Option<String>,
    ) -> Option<PlannedStmt> {
        // Inside a function layer: connect each value to the layer's boundary output pin.
        if let Some((layer, return_pins)) = self.function_return_targets.last().cloned() {
            for (index, value) in values.iter().enumerate() {
                let Some(return_pin) = return_pins.get(index).cloned() else {
                    self.result.diagnostics.push(format!(
                        "return value {} has no matching function return pin",
                        index + 1
                    ));
                    continue;
                };
                let Some(source) = self
                    .resolve_expr(value, target_layer.clone())
                    .and_then(|symbol| self.symbol_to_source(symbol, target_layer.clone()))
                else {
                    self.result.diagnostics.push(format!(
                        "return value {} is not a resolvable FlowScript value",
                        index + 1
                    ));
                    continue;
                };
                let Some(output_pin) = self.resolve_source_output_pin(&source) else {
                    self.result.diagnostics.push(format!(
                        "could not choose output pin for return value {}",
                        index + 1
                    ));
                    continue;
                };
                self.connect_commands.push(BoardCommand::ConnectPins {
                    from_node: source.node.node_ref(),
                    from_pin: output_pin,
                    to_node: layer.node_ref(),
                    to_pin: return_pin,
                    summary: Some("Connect FlowScript function return".to_string()),
                });
            }
            return None;
        }

        // Outside a function layer: an event/tool-entry `return` reverses the
        // `events_generic_return_result` sugar so agent tools and event handlers can return a value.
        self.plan_event_return(values, anchor, target_layer)
    }

    /// Reverse the `events_generic_return_result` sugar: reuse the anchored result node (or add a
    /// fresh one), wire the returned value into its `response` input, and chain it into the exec
    /// flow as a terminal statement.
    fn plan_event_return(
        &mut self,
        values: &[Expr],
        anchor: Option<&str>,
        target_layer: Option<String>,
    ) -> Option<PlannedStmt> {
        let entity = match anchor {
            Some(anchor) if find_board_node(self.existing, anchor).is_some() => {
                NodeEntity::Existing(anchor.to_string())
            }
            _ => {
                let meta =
                    self.resolve_variable_node(EVENT_RETURN_RESULT_TYPE, "returnGenericResult")?;
                self.queue_add_node(meta, target_layer.clone())
            }
        };

        if let Some(value) = values.first() {
            self.wire_return_response(&entity, value, target_layer);
        }
        Some(PlannedStmt::new(entity))
    }

    /// Wire a return value into an `events_generic_return_result` node's `response` input (literal
    /// set or data connection), reusing existing wiring on an unchanged roundtrip.
    fn wire_return_response(
        &mut self,
        entity: &NodeEntity,
        value: &Expr,
        target_layer: Option<String>,
    ) {
        let meta = match entity {
            NodeEntity::Existing(id) => find_board_node(self.existing, id).map(node_to_metadata),
            NodeEntity::New { meta, .. } => Some(meta.clone()),
            NodeEntity::Layer { .. } => None,
        };
        let Some(meta) = meta else { return };
        let Some(input) = metadata_input_pin(&meta, EVENT_RESPONSE_PIN) else {
            return;
        };

        if let Some(mut literal) = literal_expr_to_value(value) {
            self.normalize_input_value(input, &mut literal);
            self.queue_update_input(entity, input, literal, &meta);
            return;
        }

        let Some(source) = self
            .resolve_expr(value, target_layer.clone())
            .and_then(|symbol| self.symbol_to_source(symbol, target_layer))
        else {
            self.result
                .diagnostics
                .push("return value is not a resolvable FlowScript value".to_string());
            return;
        };
        let Some(output_pin) = self.resolve_source_output_pin_for_input(&source, input) else {
            return;
        };
        let already_wired = self
            .existing_source_for_input(entity, input)
            .is_some_and(|existing| {
                existing.node.node_ref() == source.node.node_ref()
                    && existing.output_pin.as_deref() == Some(output_pin.as_str())
            });
        if !already_wired {
            self.connect_commands.push(BoardCommand::ConnectPins {
                from_node: source.node.node_ref(),
                from_pin: output_pin,
                to_node: entity.node_ref(),
                to_pin: input.name.clone(),
                summary: Some("Connect FlowScript return value".to_string()),
            });
        }
    }

    fn plan_call_statement(
        &mut self,
        call: &Call,
        anchor: Option<&str>,
        target_layer: Option<String>,
    ) -> Option<NodeEntity> {
        self.plan_call_statement_with_sources(call, anchor, target_layer)
            .map(|(entity, _)| entity)
    }

    fn plan_call_statement_with_sources(
        &mut self,
        call: &Call,
        anchor: Option<&str>,
        target_layer: Option<String>,
    ) -> Option<(NodeEntity, Vec<ValueSource>)> {
        if let Some(anchor) = anchor {
            let meta = find_board_node(self.existing, anchor).map(node_to_metadata)?;
            let entity = NodeEntity::Existing(anchor.to_string());
            let input_sources = self.plan_call_arguments(call, &entity, &meta, target_layer, false);
            return Some((entity, input_sources));
        }

        if call.display.trim().is_empty() {
            return None;
        }

        self.add_call_node_with_sources(call, target_layer)
    }

    /// Resolve an event block's entry node. Arbitrary event names intentionally fall back to a
    /// simple event; that is only worth a diagnostic when declared payload params get dropped.
    fn add_entry_node(
        &mut self,
        display: &str,
        target_layer: Option<String>,
        dropped_params: bool,
    ) -> Option<NodeEntity> {
        match self.catalog.resolve_display(display) {
            Ok(meta) => Some(self.queue_add_node(meta, target_layer)),
            Err(err) => {
                for fallback in ["eventsSimple", "eventsGeneric"] {
                    if let Ok(meta) = self.catalog.resolve_display(fallback) {
                        if dropped_params {
                            self.result.diagnostics.push(format!(
                                "event `{display}` does not match a catalog entry node; created a `{fallback}` entry instead and its declared payload params were not carried over"
                            ));
                        }
                        return Some(self.queue_add_node(meta, target_layer));
                    }
                }
                self.result.diagnostics.push(err);
                None
            }
        }
    }

    fn add_call_node(&mut self, call: &Call, target_layer: Option<String>) -> Option<NodeEntity> {
        self.add_call_node_with_sources(call, target_layer)
            .map(|(entity, _)| entity)
    }

    fn add_call_node_with_sources(
        &mut self,
        call: &Call,
        target_layer: Option<String>,
    ) -> Option<(NodeEntity, Vec<ValueSource>)> {
        // `functionName(...)` calling a FlowScript `function` declaration: create a
        // `control_call_function` node targeting that function's layer instead of resolving
        // against the catalog.
        if self.planned_functions.contains_key(&call.display) {
            return self.add_function_call_node(call, target_layer);
        }
        let meta = match self.catalog.resolve_call(call) {
            Ok(meta) => meta,
            Err(err) => {
                self.result.diagnostics.push(err);
                return None;
            }
        };
        // Materialize this call's dynamic (`on_update`-generated) pins so its args resolve against
        // real pins; a no-op when no enricher is supplied (falls back to `synthesize_dynamic_input_pin`).
        let meta = self.enrich_meta(meta, call);
        let entity = self.queue_add_node(meta.clone(), target_layer.clone());

        let input_sources = self.plan_call_arguments(call, &entity, &meta, target_layer, true);

        Some((entity, input_sources))
    }

    /// Create the `control_call_function` node for a call to a FlowScript `function` declaration.
    ///
    /// The synthetic metadata mirrors what the node's `on_update` will mint once its
    /// `function_layer_id` pin is set (the layer's params as inputs, returns as outputs, exec
    /// boundary pins when the function is impure), so argument and execution wiring plan against
    /// the pins that will exist at apply time.
    fn add_function_call_node(
        &mut self,
        call: &Call,
        target_layer: Option<String>,
    ) -> Option<(NodeEntity, Vec<ValueSource>)> {
        let planned = self.planned_functions.get(&call.display).cloned()?;
        let Some(base) = self.catalog.by_type.get(CALL_FUNCTION_NODE_TYPE).cloned() else {
            self.result.diagnostics.push(format!(
                "cannot call function `{}`: `{CALL_FUNCTION_NODE_TYPE}` is missing from the catalog",
                call.display
            ));
            return None;
        };

        let mut meta = base;
        meta.friendly_name = format!("Call {}", call.display);
        // The call node's `on_update` mirrors the layer's ACTUAL boundary pins, so for an
        // existing layer trust its pins over the prescan (legacy function layers may predate
        // exec boundary pins).
        let impure = match &planned.entity {
            NodeEntity::Existing(id) => self
                .existing
                .layers
                .get(id)
                .map(|layer| {
                    layer.pins.values().any(|pin| {
                        pin.pin_type == PinType::Input
                            && pin.data_type == VariableType::Execution
                    })
                })
                .unwrap_or(planned.impure),
            _ => planned.impure,
        };
        if impure {
            meta.inputs
                .push(execution_pin_metadata(FUNCTION_EXEC_IN, "Exec In"));
            meta.outputs
                .push(execution_pin_metadata(FUNCTION_EXEC_OUT, "Exec Out"));
        }
        for param in &planned.params {
            meta.inputs.push(param_pin_metadata(param));
        }
        for ret in &planned.returns {
            meta.outputs.push(param_pin_metadata(ret));
        }

        let entity = self.queue_add_node(meta.clone(), target_layer.clone());
        if let Some(input) = metadata_input_pin(&meta, FUNCTION_LAYER_ID_PIN) {
            // For a new layer this is its `$n` ref; the applier resolves it to the real layer id.
            self.queue_update_input(
                &entity,
                input,
                flow_like_types::Value::String(planned.entity.node_ref()),
                &meta,
            );
        }

        let input_sources = self.plan_call_arguments(call, &entity, &meta, target_layer, true);
        Some((entity, input_sources))
    }

    fn plan_call_arguments(
        &mut self,
        call: &Call,
        entity: &NodeEntity,
        meta: &NodeMetadata,
        target_layer: Option<String>,
        include_direct_literals: bool,
    ) -> Vec<ValueSource> {
        let mut input_sources = Vec::new();
        let mut same_name_seen: HashMap<&str, usize> = HashMap::new();
        for arg in &call.args {
            // Multi-pins (several inputs sharing one name) pair positionally with the
            // same-named args, mirroring the order lowering emitted them in.
            let occurrence = {
                let counter = same_name_seen.entry(arg.name.as_str()).or_insert(0);
                let index = *counter;
                *counter += 1;
                index
            };
            if occurrence > 0
                && matches!(entity, NodeEntity::New { .. })
                && metadata_input_pin_at(meta, &arg.name, occurrence).is_some()
            {
                // Board commands address pins by NAME, so a second same-named pin on a node
                // created in this same batch cannot be targeted yet — surface it instead of
                // silently collapsing both args onto the first pin.
                self.result.diagnostics.push(format!(
                    "`{}` uses the multi-pin argument `{}` more than once; repeated same-named arguments on a NEW node are not supported through FlowScript yet — configure the extra pin afterwards or via emit_commands",
                    call.display, arg.name
                ));
                continue;
            }
            // Pins minted by a node's `on_update` (for example each `{placeholder}` of a
            // `string_format` node) are absent from the STATIC catalog metadata used to plan a NEW
            // node, so they must be predicted from the call's own config args. `synthesized_pin`
            // backs the `&PinMetadata` borrow for such a predicted dynamic pin.
            let synthesized_pin;
            let input = match metadata_input_pin_at(meta, &arg.name, occurrence) {
                Some(pin) => pin,
                None => {
                    // `tools:`/`fnRefs:` carry a node's function references, not pin values — they
                    // are not board pins. For a NEWLY added node, materialize them as a
                    // `SetNodeFunctionRefs` command (the applier resolves each named target once the
                    // referenced functions/events exist). For an EXISTING node, leave its
                    // references untouched: an unchanged document round-trips to a clean no-op, and
                    // rewriting them would need exact ref→entry-node resolution against the live
                    // board.
                    if is_synthetic_fn_ref_arg(arg) {
                        if let NodeEntity::New { .. } = entity {
                            let refs = synthetic_fn_ref_targets(arg);
                            if !refs.is_empty() {
                                self.fn_ref_commands
                                    .push(BoardCommand::SetNodeFunctionRefs {
                                        node_id: entity.node_ref(),
                                        fn_refs: refs,
                                        summary: Some(format!(
                                            "Register {} on {}",
                                            arg.name, meta.friendly_name
                                        )),
                                    });
                            }
                        }
                        continue;
                    }
                    // A dynamic pin the node's `on_update` will add from its config (for example a
                    // `string_format` placeholder that appears in the format string): synthesize a
                    // permissive Generic pin so its value/connection is still planned. Apply sets
                    // the config pin first, runs `on_update`, then resolves the wire against the
                    // now-live pin (see `plan()`'s update-before-connect ordering).
                    match synthesize_dynamic_input_pin(meta, call, arg, entity, self.existing) {
                        Some(pin) => {
                            synthesized_pin = pin;
                            &synthesized_pin
                        }
                        None => {
                            self.result.diagnostics.push(format!(
                                "node `{}` has no input pin named `{}`; skipped that argument",
                                call.display, arg.name
                            ));
                            continue;
                        }
                    }
                }
            };

            if let Some(mut value) = literal_expr_to_value(&arg.value) {
                self.normalize_input_value(input, &mut value);
                if include_direct_literals {
                    self.queue_update_input(entity, input, value, meta);
                }
                continue;
            }

            let Some(source) =
                self.resolve_expr_for_argument(&arg.value, entity, input, target_layer.clone())
            else {
                // Expression forms v1 cannot resolve (ternaries, binaries, event returns) but
                // whose input is already wired on the board: keep the existing wiring silently —
                // the rendered text CAME from that wiring, so this is not an authoring error.
                if let Some(existing_source) = self.existing_source_for_input(entity, input) {
                    input_sources.push(existing_source);
                    continue;
                }
                self.result.diagnostics.push(format!(
                    "argument `{}` on `{}` is not a literal or resolvable node output; skipped connection",
                    arg.name, call.display
                ));
                continue;
            };
            let source = match source {
                SymbolValue::Literal(mut value) => {
                    self.normalize_input_value(input, &mut value);
                    self.queue_update_input(entity, input, value, meta);
                    continue;
                }
                SymbolValue::VariableRef { variable_id }
                    if is_variable_ref_pin_name(&input.name) =>
                {
                    self.queue_update_input(
                        entity,
                        input,
                        flow_like_types::Value::String(variable_id),
                        meta,
                    );
                    continue;
                }
                SymbolValue::Source(source) => source,
                SymbolValue::VariableRef { variable_id } => {
                    let Some(source) =
                        self.add_variable_get_source(&variable_id, target_layer.clone())
                    else {
                        self.result.diagnostics.push(format!(
                            "could not create variable read for argument `{}` on `{}`",
                            arg.name, call.display
                        ));
                        continue;
                    };
                    source
                }
            };
            let Some(output_pin) = self.resolve_source_output_pin_for_input(&source, input) else {
                self.result.diagnostics.push(format!(
                    "could not choose an output pin for argument `{}` on `{}`",
                    arg.name, call.display
                ));
                continue;
            };
            // Roundtrip no-op: the board may already carry exactly this data edge — re-emitting
            // it would flood every reconcile of an unchanged document with ConnectPins commands.
            let already_wired =
                self.existing_source_for_input(entity, input)
                    .is_some_and(|existing_source| {
                        existing_source.node.node_ref() == source.node.node_ref()
                            && existing_source.output_pin.as_deref() == Some(output_pin.as_str())
                    });
            if !already_wired {
                self.connect_commands.push(BoardCommand::ConnectPins {
                    from_node: source.node.node_ref(),
                    from_pin: output_pin.clone(),
                    to_node: entity.node_ref(),
                    to_pin: input.name.clone(),
                    summary: Some(format!("Connect {} into {}", arg.name, meta.friendly_name)),
                });
            }
            input_sources.push(ValueSource {
                node: source.node,
                output_pin: Some(output_pin),
            });
        }
        input_sources
    }

    fn resolve_expr_for_argument(
        &mut self,
        expr: &Expr,
        target_entity: &NodeEntity,
        input: &PinMetadata,
        target_layer: Option<String>,
    ) -> Option<SymbolValue> {
        if let Some(source) = self.existing_source_for_input(target_entity, input)
            && let Some(symbol) =
                self.resolve_expr_using_existing_source(expr, source, target_layer.clone())
        {
            return Some(symbol);
        }

        self.resolve_expr(expr, target_layer)
    }

    fn existing_source_for_input(
        &self,
        target_entity: &NodeEntity,
        input: &PinMetadata,
    ) -> Option<ValueSource> {
        let NodeEntity::Existing(node_id) = target_entity else {
            return None;
        };
        let node = find_board_node(self.existing, node_id)?;
        self.board_index.data_source_for_input(node, &input.name)
    }

    fn resolve_expr_using_existing_source(
        &mut self,
        expr: &Expr,
        source: ValueSource,
        target_layer: Option<String>,
    ) -> Option<SymbolValue> {
        match expr {
            Expr::Call(call) => self.reuse_existing_call_source(call, source, None, target_layer),
            Expr::Field { base, pin } => {
                if let Expr::Call(call) = base.as_ref() {
                    self.reuse_existing_call_source(call, source, Some(pin), target_layer)
                } else {
                    None
                }
            }
            // Sugared data sources: these text forms lower FROM a specific node shape, and
            // when the consumer's existing source IS that node, it must be reused. Falling
            // back to `resolve_expr` would materialize a duplicate node on every apply, and
            // resolving only the base would silently drop the field/index selection.
            Expr::Ref(name) => self.reuse_existing_variable_get(name, source),
            Expr::Member { base, field } => {
                self.reuse_existing_member_source(base, field, source, target_layer)
            }
            Expr::Index { base, index } => {
                self.reuse_existing_index_source(base, index, source, target_layer)
            }
            Expr::Ternary {
                cond,
                then,
                otherwise,
            } => self.reuse_existing_select_source(cond, then, otherwise, source, target_layer),
            _ => None,
        }
    }

    /// Reuse an existing `variable_get` node feeding this input when the text ref names the
    /// same variable it reads.
    fn reuse_existing_variable_get(
        &mut self,
        name: &str,
        source: ValueSource,
    ) -> Option<SymbolValue> {
        let NodeEntity::Existing(node_id) = &source.node else {
            return None;
        };
        let node = find_board_node(self.existing, node_id)?;
        if node.name != "variable_get" {
            return None;
        }
        let SymbolValue::VariableRef { variable_id } = self.lookup_symbol(name)? else {
            return None;
        };
        let configured = node_pin_literal_string(node, "var_ref")?;
        (configured == variable_id).then_some(SymbolValue::Source(source))
    }

    /// Reuse the existing struct/array accessor node a `base.field` member access lowered
    /// from (`struct_get`, `struct_break`, or `array_length`). The base is recursed so
    /// literal edits deeper in the chain still apply.
    fn reuse_existing_member_source(
        &mut self,
        base: &Expr,
        field: &str,
        source: ValueSource,
        target_layer: Option<String>,
    ) -> Option<SymbolValue> {
        let NodeEntity::Existing(node_id) = &source.node else {
            return None;
        };
        let node = find_board_node(self.existing, node_id)?;
        let base_input = match node.name.as_str() {
            "struct_get"
                if node_pin_literal_string(node, "field").as_deref() == Some(field) =>
            {
                "struct"
            }
            "struct_break"
                if source
                    .output_pin
                    .as_deref()
                    .and_then(|pin| pin.strip_prefix(super::lower::BREAK_STRUCT_PREFIX))
                    == Some(field) =>
            {
                "struct_in"
            }
            "array_length" if field == "length" => "array",
            _ => return None,
        };
        if let Some(base_source) = self.board_index.data_source_for_input(node, base_input) {
            let _ = self.resolve_expr_using_existing_source(base, base_source, target_layer);
        }
        Some(SymbolValue::Source(source))
    }

    /// Reuse the existing `array_get` node a `base[index]` access lowered from. A changed
    /// literal index becomes a pin update on the same node.
    fn reuse_existing_index_source(
        &mut self,
        base: &Expr,
        index: &Expr,
        source: ValueSource,
        target_layer: Option<String>,
    ) -> Option<SymbolValue> {
        let NodeEntity::Existing(node_id) = &source.node else {
            return None;
        };
        let node = find_board_node(self.existing, node_id)?;
        if node.name != "array_get" || source.output_pin.as_deref() != Some("element") {
            return None;
        }
        if let Some(base_source) = self.board_index.data_source_for_input(node, "array_in") {
            let _ = self.resolve_expr_using_existing_source(base, base_source, target_layer);
        }
        if let Some(value) = literal_expr_to_value(index) {
            let meta = node_to_metadata(node);
            if let Some(pin) = metadata_input_pin(&meta, "index") {
                let entity = NodeEntity::Existing(node_id.clone());
                self.queue_update_input(&entity, pin, value, &meta);
            }
        }
        Some(SymbolValue::Source(source))
    }

    /// Reuse the existing `utils_types_select` node a `cond ? a : b` ternary lowered from,
    /// diffing its literal inputs and recursing into wired ones.
    fn reuse_existing_select_source(
        &mut self,
        cond: &Expr,
        then: &Expr,
        otherwise: &Expr,
        source: ValueSource,
        target_layer: Option<String>,
    ) -> Option<SymbolValue> {
        let NodeEntity::Existing(node_id) = &source.node else {
            return None;
        };
        let node = find_board_node(self.existing, node_id)?;
        if node.name != "utils_types_select" {
            return None;
        }
        let meta = node_to_metadata(node);
        for (pin_name, expr) in [("condition", cond), ("a", then), ("b", otherwise)] {
            if let Some(value) = literal_expr_to_value(expr) {
                if let Some(pin) = metadata_input_pin(&meta, pin_name) {
                    let entity = NodeEntity::Existing(node_id.clone());
                    self.queue_update_input(&entity, pin, value, &meta);
                }
            } else if let Some(sub_source) = self.board_index.data_source_for_input(node, pin_name)
            {
                let _ =
                    self.resolve_expr_using_existing_source(expr, sub_source, target_layer.clone());
            }
        }
        Some(SymbolValue::Source(source))
    }

    fn reuse_existing_call_source(
        &mut self,
        call: &Call,
        source: ValueSource,
        requested_output: Option<&str>,
        target_layer: Option<String>,
    ) -> Option<SymbolValue> {
        let NodeEntity::Existing(source_node_id) = &source.node else {
            return None;
        };
        let source_node = find_board_node(self.existing, source_node_id)?;
        // Always describe the reused node by ITS OWN pins: catalog metadata is a per-type sample
        // and misses instance pins of dynamic nodes (string_format placeholders, added pins).
        // A call to a declared FlowScript function reuses the existing `control_call_function`
        // node that targets that function's layer (catalog resolution can never match it, and
        // recreating it on every reconcile would duplicate call nodes).
        let is_function_call_reuse = source_node.name == CALL_FUNCTION_NODE_TYPE
            && self.planned_functions.get(&call.display).is_some_and(|planned| {
                node_pin_literal_string(source_node, FUNCTION_LAYER_ID_PIN)
                    .is_some_and(|layer_id| layer_id == planned.entity.node_ref())
            });
        let meta = if is_function_call_reuse {
            node_to_metadata(source_node)
        } else {
            match self.catalog.resolve_call(call) {
                Ok(meta) if meta.name == source_node.name => node_to_metadata(source_node),
                Ok(_) => return None,
                Err(_) if call_matches_node(call, source_node) => node_to_metadata(source_node),
                Err(_) => return None,
            }
        };
        let entity = NodeEntity::Existing(source_node_id.clone());
        self.plan_call_arguments(call, &entity, &meta, target_layer, true);

        let output_pin = requested_output
            .and_then(|pin| self.resolve_entity_output_pin(&entity, Some(pin)))
            .or(source.output_pin)
            .or_else(|| self.resolve_entity_output_pin(&entity, None));

        Some(SymbolValue::Source(ValueSource {
            node: entity,
            output_pin,
        }))
    }

    fn normalize_input_value(&mut self, input: &PinMetadata, value: &mut flow_like_types::Value) {
        if !is_variable_ref_pin_name(&input.name) {
            return;
        }

        let flow_like_types::Value::String(raw) = value else {
            return;
        };

        if let Some(variable_id) = self.variable_refs.resolve(raw) {
            *raw = variable_id;
            return;
        }

        if self.unresolved_variable_refs.insert(raw.clone()) {
            self.result.diagnostics.push(format!(
                "variable reference `{raw}` does not resolve to a board or FlowScript variable; declare it as a top-level FlowScript variable so it can be created"
            ));
        }
    }

    fn queue_update_input(
        &mut self,
        entity: &NodeEntity,
        input: &PinMetadata,
        value: flow_like_types::Value,
        meta: &NodeMetadata,
    ) {
        if let NodeEntity::Existing(node_id) = entity
            && let Some(node) = find_board_node(self.existing, node_id)
            && let Some(pin) = find_input_pin(node, &input.name)
        {
            let current = pin.default_value.as_deref().and_then(|bytes| {
                flow_like_types::json::from_slice::<flow_like_types::Value>(bytes).ok()
            });
            if current.as_ref() == Some(&value) {
                return;
            }
        }

        self.update_commands.push(BoardCommand::UpdateNodePin {
            node_id: entity.node_ref(),
            pin_id: input.name.clone(),
            value,
            summary: Some(format!("Set {} on {}", input.name, meta.friendly_name)),
        });
    }

    /// Warn (non-blocking) about newly added impure nodes — nodes with an execution input — that end
    /// up with no incoming execution edge and would therefore never run. Event/entry nodes have no
    /// execution input and are skipped implicitly; a function body's first node is exempt (no entry
    /// to wire from yet); existing anchored nodes are never recorded, so only new nodes are checked.
    fn check_dangling_impure_execution(&mut self) {
        let nodes = std::mem::take(&mut self.new_impure_nodes);
        for (ref_id, exec_in, friendly_name) in &nodes {
            if self.exec_check_exempt.contains(ref_id) {
                continue;
            }
            let connected = self.connect_commands.iter().any(|command| {
                matches!(
                    command,
                    BoardCommand::ConnectPins { to_node, to_pin, .. }
                        if to_node == ref_id && to_pin == exec_in
                )
            });
            if !connected {
                self.result.diagnostics.push(format!(
                    "node `{friendly_name}` has no incoming execution connection and will not run; wire its execution input from the previous statement's execution output"
                ));
            }
        }
    }

    fn queue_add_node(&mut self, meta: NodeMetadata, target_layer: Option<String>) -> NodeEntity {
        let ref_id = format!("${}", self.next_ref);
        self.next_ref += 1;
        if let Some(exec_in) = metadata_exec_input_pin(&meta) {
            self.new_impure_nodes
                .push((ref_id.clone(), exec_in, meta.friendly_name.clone()));
        }
        let position = self.next_position();
        self.add_commands.push(BoardCommand::AddNode {
            node_type: meta.name.clone(),
            ref_id: Some(ref_id.clone()),
            position: Some(position),
            friendly_name: None,
            target_layer,
            summary: Some(format!("Add {}", meta.friendly_name)),
        });
        NodeEntity::New { ref_id, meta }
    }

    fn next_position(&mut self) -> NodePosition {
        let (base_x, base_y) = self.rightmost_existing_position();
        let position = NodePosition {
            x: base_x + 260.0 * (self.next_position as f64 + 1.0),
            y: base_y + 120.0 * ((self.next_position / 4) as f64),
        };
        self.next_position += 1;
        position
    }

    fn rightmost_existing_position(&self) -> (f64, f64) {
        let mut rightmost: Option<(f32, f32)> = None;
        for node in all_board_nodes(self.existing) {
            if let Some((x, y, _)) = node.coordinates {
                if rightmost.map_or(true, |(rx, _)| x > rx) {
                    rightmost = Some((x, y));
                }
            }
        }
        rightmost
            .map(|(x, y)| (x as f64, y as f64))
            .unwrap_or((0.0, 200.0))
    }

    /// Wire `previous` into `current`'s exec input. Returns the `(from_node, from_pin)`
    /// edge that was connected, or `None` when no connection was made.
    ///
    /// `insertion_origin` is the existing predecessor edge the current insertion streak
    /// branched off from. When new nodes were spliced before an existing target, ONLY that
    /// edge is disconnected — an exec input is a legal fan-in point, and the other incoming
    /// edges belong to unrelated events/branches that must stay wired.
    fn connect_exec(
        &mut self,
        previous: &ExecCursor,
        current: &NodeEntity,
        insertion_origin: Option<&(String, String)>,
    ) -> Option<(String, String)> {
        // Existing→existing pairs keep their board wiring (v1 does not rewrite exec edges) —
        // check FIRST so unchanged roundtrips don't emit no-continuation-policy diagnostics for
        // multi-output nodes whose successors are already wired.
        if matches!(previous.entity, NodeEntity::Existing(_))
            && matches!(current, NodeEntity::Existing(_))
        {
            return None;
        }

        let Some(from_pin) = previous
            .output_pin
            .clone()
            .or_else(|| self.entity_exec_output_pin(&previous.entity))
        else {
            let outputs = self.entity_exec_output_pins(&previous.entity);
            if outputs.len() > 1 {
                self.result.diagnostics.push(format!(
                    "node `{}` has multiple execution outputs ({}) and no default continuation policy; add an explicit policy before auto-wiring sequential FlowScript calls",
                    previous.entity.node_ref(),
                    outputs.join(", ")
                ));
            }
            return None;
        };
        let Some(to_pin) = self.entity_exec_input_pin(current) else {
            return None;
        };

        if let Some((origin_node, origin_pin)) = insertion_origin
            && let NodeEntity::Existing(node_id) = current
            && let Some(node) = find_board_node(self.existing, node_id)
        {
            for (from_node, from_pin) in self.board_index.exec_incoming_edges(node, &to_pin) {
                if &from_node == origin_node && &from_pin == origin_pin {
                    self.disconnect_commands.push(BoardCommand::DisconnectPins {
                        from_node,
                        from_pin,
                        to_node: node_id.clone(),
                        to_pin: to_pin.clone(),
                        summary: Some(format!("Rewire execution into {}", node.friendly_name)),
                    });
                }
            }
        }

        self.connect_commands.push(BoardCommand::ConnectPins {
            from_node: previous.entity.node_ref(),
            from_pin: from_pin.clone(),
            to_node: current.node_ref(),
            to_pin,
            summary: Some("Connect FlowScript execution order".to_string()),
        });
        Some((previous.entity.node_ref(), from_pin))
    }

    fn preferred_exec_output_for_input_sources(
        &self,
        previous: &NodeEntity,
        input_sources: &[ValueSource],
    ) -> Option<String> {
        let previous_ref = previous.node_ref();
        input_sources.iter().find_map(|source| {
            if source.node.node_ref() != previous_ref {
                return None;
            }
            let output_pin = source.output_pin.as_deref()?;
            self.exec_output_for_data_output(previous, output_pin)
        })
    }

    fn exec_output_for_data_output(&self, entity: &NodeEntity, output_pin: &str) -> Option<String> {
        if !is_streaming_data_output_pin_name(output_pin) {
            return None;
        }
        self.entity_exec_output_pin_named(
            entity,
            &["on_stream", "on_chunk", "for_chunk", "on_delta", "on_token"],
        )
    }

    fn entity_exec_input_pin(&self, entity: &NodeEntity) -> Option<String> {
        match entity {
            NodeEntity::Existing(id) => find_board_node(self.existing, id).and_then(exec_input_pin),
            NodeEntity::New { meta, .. } => metadata_exec_input_pin(meta),
            NodeEntity::Layer { .. } => None,
        }
    }

    fn entity_exec_output_pin(&self, entity: &NodeEntity) -> Option<String> {
        match entity {
            NodeEntity::Existing(id) => {
                find_board_node(self.existing, id).and_then(exec_output_pin)
            }
            NodeEntity::New { meta, .. } => metadata_exec_output_pin(meta),
            NodeEntity::Layer { .. } => None,
        }
    }

    fn entity_exec_output_pins(&self, entity: &NodeEntity) -> Vec<String> {
        match entity {
            NodeEntity::Existing(id) => find_board_node(self.existing, id)
                .map(|node| {
                    let mut pins: Vec<&Pin> = node
                        .pins
                        .values()
                        .filter(|p| p.pin_type == PinType::Output && is_exec_pin(p))
                        .collect();
                    pins.sort_by_key(|p| p.index);
                    pins.into_iter().map(|p| p.name.clone()).collect()
                })
                .unwrap_or_default(),
            NodeEntity::New { meta, .. } => meta
                .outputs
                .iter()
                .filter(|p| p.data_type == "Execution")
                .map(|p| p.name.clone())
                .collect(),
            NodeEntity::Layer { .. } => Vec::new(),
        }
    }

    /// Resolve a branch arm's label to an exec output pin. Labels come in two shapes: lowered
    /// text uses raw pin names (`exec_out_exists`), while the if/else sugar uses `True`/`False`
    /// — so the camelized label is tried as well (`True` → `true`).
    fn resolve_arm_exec_pin(&self, entity: &NodeEntity, label: &str) -> Option<String> {
        let camel = to_camel_case(label);
        self.entity_exec_output_pin_named(entity, &[label, camel.as_str()])
    }

    fn entity_exec_output_pin_named(&self, entity: &NodeEntity, names: &[&str]) -> Option<String> {
        match entity {
            NodeEntity::Existing(id) => {
                let node = find_board_node(self.existing, id)?;
                names
                    .iter()
                    .find_map(|name| find_output_pin(node, name).filter(|pin| is_exec_pin(pin)))
                    .map(|pin| pin.name.clone())
            }
            NodeEntity::New { meta, .. } => names
                .iter()
                .find_map(|name| {
                    meta.outputs.iter().find(|pin| {
                        pin.data_type == "Execution" && metadata_pin_name_matches(pin, name)
                    })
                })
                .map(|pin| pin.name.clone()),
            NodeEntity::Layer { .. } => None,
        }
    }

    /// Lower a struct-field access (`<source>.field`, where `<source>` already selected an output
    /// pin) to a `struct_get` node: `struct_get({ struct: <source>, field: "field" }).value`. This
    /// makes dot-notation field access on a node output resolve. Returns `None` (leaving the original
    /// diagnostic to fire) if the catalog has no `struct_get` node.
    fn lower_struct_field_access(
        &mut self,
        base: ValueSource,
        field: &str,
        target_layer: Option<String>,
    ) -> Option<SymbolValue> {
        let probe = Call {
            node_type: "struct_get".to_string(),
            display: "structGet".to_string(),
            args: Vec::new(),
            anchor: None,
        };
        let meta = self.catalog.resolve_call(&probe).ok()?;
        let entity = self.queue_add_node(meta.clone(), target_layer);

        if let Some(field_pin) = metadata_input_pin(&meta, "field") {
            self.queue_update_input(
                &entity,
                field_pin,
                flow_like_types::Value::String(field.to_string()),
                &meta,
            );
        }

        if let Some(struct_pin) = metadata_input_pin(&meta, "struct") {
            let from_pin = base
                .output_pin
                .clone()
                .or_else(|| self.resolve_entity_output_pin(&base.node, None));
            if let Some(from_pin) = from_pin {
                self.connect_commands.push(BoardCommand::ConnectPins {
                    from_node: base.node.node_ref(),
                    from_pin,
                    to_node: entity.node_ref(),
                    to_pin: struct_pin.name.clone(),
                    summary: Some(format!("Read struct field `{field}`")),
                });
            }
        }

        let output = self.resolve_entity_output_pin(&entity, Some("value"))?;
        Some(SymbolValue::Source(ValueSource {
            node: entity,
            output_pin: Some(output),
        }))
    }

    /// Materialize an `array_length` node for a `.length` member access.
    fn lower_array_length_access(
        &mut self,
        base: ValueSource,
        target_layer: Option<String>,
    ) -> Option<SymbolValue> {
        let probe = Call {
            node_type: "array_length".to_string(),
            display: "arrayLength".to_string(),
            args: Vec::new(),
            anchor: None,
        };
        let meta = self.catalog.resolve_call(&probe).ok()?;
        let entity = self.queue_add_node(meta.clone(), target_layer);

        if let Some(array_pin) = metadata_input_pin(&meta, "array") {
            let from_pin = base
                .output_pin
                .clone()
                .or_else(|| self.resolve_entity_output_pin(&base.node, None));
            if let Some(from_pin) = from_pin {
                self.connect_commands.push(BoardCommand::ConnectPins {
                    from_node: base.node.node_ref(),
                    from_pin,
                    to_node: entity.node_ref(),
                    to_pin: array_pin.name.clone(),
                    summary: Some("Read array length".to_string()),
                });
            }
        }

        let output = self
            .resolve_entity_output_pin(&entity, Some("length"))
            .or_else(|| self.resolve_entity_output_pin(&entity, None));
        Some(SymbolValue::Source(ValueSource {
            node: entity,
            output_pin: output,
        }))
    }

    /// Materialize an `array_get` node for a `base[index]` access, reading its `element`
    /// output. The synthetic call routes base/index through the standard argument planner.
    fn lower_array_index_access(
        &mut self,
        base: &Expr,
        index: &Expr,
        target_layer: Option<String>,
    ) -> Option<SymbolValue> {
        let call = Call {
            node_type: "array_get".to_string(),
            display: "arrayGet".to_string(),
            args: vec![
                Arg {
                    name: "array_in".to_string(),
                    value: base.clone(),
                },
                Arg {
                    name: "index".to_string(),
                    value: index.clone(),
                },
            ],
            anchor: None,
        };
        let entity = self.add_call_node(&call, target_layer)?;
        let output = self
            .resolve_entity_output_pin(&entity, Some("element"))
            .or_else(|| self.resolve_entity_output_pin(&entity, None));
        Some(SymbolValue::Source(ValueSource {
            node: entity,
            output_pin: output,
        }))
    }

    /// Drop a statement's own primary call node from the exec-splice queue.
    ///
    /// A bare `x = impureCall(...)` reassignment (or a `let`/`const` alias such as
    /// `x = call(...).pin`) resolves its RHS through [`Self::resolve_expr`], whose `Expr::Call`
    /// arm queues every impure call for exec-splicing — the machinery that threads impure calls
    /// buried in *arguments* into the execution chain ahead of the consuming statement. But the
    /// RHS's own top-level call is the statement's primary node, and `plan_block` already
    /// exec-wires it as the `current` statement. Leaving it queued wires it twice, and because the
    /// splice makes it `previous_exec` immediately before it becomes `current`, `connect_exec`
    /// runs its execution output straight back into its own input — a self-connection that aborts
    /// apply with "Cannot connect a node to itself". Removing it leaves the statement path as its
    /// single, correct execution wiring; genuine argument-position splices stay queued.
    fn undefer_statement_call_splice(&mut self, entity: &NodeEntity) {
        let ref_id = entity.node_ref();
        self.pending_exec_splices
            .retain(|splice| splice.node_ref() != ref_id);
    }

    fn resolve_expr(&mut self, expr: &Expr, target_layer: Option<String>) -> Option<SymbolValue> {
        if let Some(value) = literal_expr_to_value(expr) {
            return Some(SymbolValue::Literal(value));
        }

        match expr {
            Expr::Ref(name) => self.lookup_symbol(name),
            Expr::Field { base, pin } => {
                let mut source = match self.resolve_expr(base, target_layer.clone())? {
                    SymbolValue::Source(source) => source,
                    SymbolValue::VariableRef { variable_id } => {
                        self.add_variable_get_source(&variable_id, target_layer.clone())?
                    }
                    SymbolValue::Literal(_) => return None,
                };
                match self.resolve_entity_output_pin(&source.node, Some(pin)) {
                    Some(output) => {
                        source.output_pin = Some(output);
                        Some(SymbolValue::Source(source))
                    }
                    // The name is not an output pin: models routinely write camelCase struct
                    // DATA keys (`mail.subject`), which the parser classifies as Field. Fall
                    // back to the struct_get lowering the Member arm uses instead of dropping
                    // the connection.
                    None => self.lower_struct_field_access(source, pin, target_layer),
                }
            }
            Expr::Call(call) => self.add_call_node(call, target_layer).map(|node| {
                // Impure calls in expression position only get data wiring here; queue them so
                // plan_block splices them into the exec chain before the consuming statement.
                if self.entity_exec_input_pin(&node).is_some() {
                    self.pending_exec_splices.push(node.clone());
                }
                SymbolValue::Source(ValueSource {
                    node,
                    output_pin: None,
                })
            }),
            Expr::Member { base, field } => {
                // `base.field` is the text form of struct_get / array_length; rebuild the
                // accessor node instead of silently dropping the selection (which would
                // wire the consumer straight to the base value).
                let base_symbol = self.resolve_expr(base, target_layer.clone())?;
                let base_source = self.symbol_to_source(base_symbol, target_layer.clone())?;
                if field == "length" {
                    self.lower_array_length_access(base_source, target_layer)
                } else {
                    self.lower_struct_field_access(base_source, field, target_layer)
                }
            }
            Expr::Index { base, index } => {
                self.lower_array_index_access(base, index, target_layer)
            }
            Expr::Object(_) | Expr::Array(_) | Expr::Ternary { .. } | Expr::Binary { .. } => None,
            Expr::Literal(_) => None,
        }
    }

    fn symbol_to_source(
        &mut self,
        symbol: SymbolValue,
        target_layer: Option<String>,
    ) -> Option<ValueSource> {
        match symbol {
            SymbolValue::Source(source) => Some(source),
            SymbolValue::VariableRef { variable_id } => {
                self.add_variable_get_source(&variable_id, target_layer)
            }
            SymbolValue::Literal(_) => None,
        }
    }

    fn add_variable_get_source(
        &mut self,
        variable_id: &str,
        target_layer: Option<String>,
    ) -> Option<ValueSource> {
        let meta = self.resolve_variable_node("variable_get", "variableGet")?;
        let entity = self.queue_add_node(meta.clone(), target_layer);
        if let Some(input) = metadata_input_pin(&meta, "var_ref") {
            self.update_commands.push(BoardCommand::UpdateNodePin {
                node_id: entity.node_ref(),
                pin_id: input.name.clone(),
                value: flow_like_types::Value::String(variable_id.to_string()),
                summary: Some("Select FlowScript variable".to_string()),
            });
        }
        Some(ValueSource {
            node: entity,
            output_pin: Some("value_ref".to_string()),
        })
    }

    fn add_variable_set_node(
        &mut self,
        variable_id: &str,
        value: &Expr,
        target_layer: Option<String>,
    ) -> Option<NodeEntity> {
        let meta = self.resolve_variable_node("variable_set", "variableSet")?;
        let entity = self.queue_add_node(meta.clone(), target_layer.clone());

        if let Some(input) = metadata_input_pin(&meta, "var_ref") {
            self.update_commands.push(BoardCommand::UpdateNodePin {
                node_id: entity.node_ref(),
                pin_id: input.name.clone(),
                value: flow_like_types::Value::String(variable_id.to_string()),
                summary: Some("Select FlowScript variable".to_string()),
            });
        }

        let Some(input) = metadata_input_pin(&meta, "value_in") else {
            self.result
                .diagnostics
                .push("variable_set has no value input pin".to_string());
            return Some(entity);
        };

        if let Some(literal) = literal_expr_to_value(value) {
            self.update_commands.push(BoardCommand::UpdateNodePin {
                node_id: entity.node_ref(),
                pin_id: input.name.clone(),
                value: literal,
                summary: Some("Set FlowScript variable value".to_string()),
            });
            return Some(entity);
        }

        let Some(source) = self
            .resolve_expr(value, target_layer.clone())
            .and_then(|symbol| self.symbol_to_source(symbol, target_layer))
        else {
            self.result.diagnostics.push(format!(
                "assignment to variable `{variable_id}` is not a resolvable value"
            ));
            return Some(entity);
        };

        if let Some(output_pin) = self.resolve_source_output_pin_for_input(&source, input) {
            self.connect_commands.push(BoardCommand::ConnectPins {
                from_node: source.node.node_ref(),
                from_pin: output_pin,
                to_node: entity.node_ref(),
                to_pin: input.name.clone(),
                summary: Some("Set FlowScript variable value".to_string()),
            });
        }

        Some(entity)
    }

    fn plan_existing_variable_set_node(
        &mut self,
        entity: &NodeEntity,
        variable_id: &str,
        value: &Expr,
        target_layer: Option<String>,
    ) {
        let NodeEntity::Existing(node_id) = entity else {
            return;
        };
        let Some(node) = find_board_node(self.existing, node_id) else {
            return;
        };
        let meta = node_to_metadata(node);

        if let Some(input) = metadata_input_pin(&meta, "var_ref") {
            self.queue_update_input(
                entity,
                input,
                flow_like_types::Value::String(variable_id.to_string()),
                &meta,
            );
        }

        let Some(input) = metadata_input_pin(&meta, "value_in")
            .or_else(|| metadata_input_pin(&meta, "new_value"))
            .or_else(|| metadata_input_pin(&meta, "value"))
        else {
            self.result.diagnostics.push(format!(
                "variable set node `{node_id}` has no value input pin"
            ));
            return;
        };

        if let Some(mut literal) = literal_expr_to_value(value) {
            self.normalize_input_value(input, &mut literal);
            self.queue_update_input(entity, input, literal, &meta);
            return;
        }

        let Some(source) = self
            .resolve_expr_for_argument(value, entity, input, target_layer.clone())
            .and_then(|symbol| self.symbol_to_source(symbol, target_layer))
        else {
            self.result.diagnostics.push(format!(
                "assignment to variable `{variable_id}` is not a resolvable value"
            ));
            return;
        };

        if let Some(output_pin) = self.resolve_source_output_pin_for_input(&source, input) {
            self.connect_commands.push(BoardCommand::ConnectPins {
                from_node: source.node.node_ref(),
                from_pin: output_pin,
                to_node: entity.node_ref(),
                to_pin: input.name.clone(),
                summary: Some("Set FlowScript variable value".to_string()),
            });
        }
    }

    fn resolve_variable_node(&mut self, node_type: &str, display: &str) -> Option<NodeMetadata> {
        if let Some(meta) = self.catalog.by_type.get(node_type) {
            return Some(meta.clone());
        }
        match self.catalog.resolve_display(display) {
            Ok(meta) => Some(meta),
            Err(err) => {
                self.result.diagnostics.push(err);
                None
            }
        }
    }

    fn create_local_variable(
        &mut self,
        name: &str,
        value: &Expr,
        target_layer: Option<String>,
    ) -> String {
        let variable_id = generated_variable_id(name);
        self.variable_refs.insert(&variable_id, name);
        let default_value = literal_expr_to_value(value);
        let (data_type, value_type) = infer_variable_types(default_value.as_ref());
        self.add_commands.push(BoardCommand::CreateVariable {
            variable_id: Some(variable_id.clone()),
            name: name.to_string(),
            data_type,
            value_type,
            default_value,
            description: None,
            category: None,
            schema: None,
            exposed: Some(false),
            secret: Some(false),
            editable: Some(true),
            runtime_configured: Some(false),
            target_layer,
            summary: Some(format!("Create local FlowScript variable {name}")),
        });
        variable_id
    }

    fn variable_id_for_assignment_target(&mut self, target: &str) -> Option<String> {
        match self.lookup_symbol(target) {
            Some(SymbolValue::VariableRef { variable_id }) => Some(variable_id),
            _ => None,
        }
    }

    fn resolve_source_output_pin(&self, source: &ValueSource) -> Option<String> {
        if source.output_pin.is_some() {
            return source.output_pin.clone();
        }
        self.resolve_entity_output_pin(&source.node, None)
    }

    fn resolve_source_output_pin_for_input(
        &self,
        source: &ValueSource,
        input: &PinMetadata,
    ) -> Option<String> {
        if source.output_pin.is_some() {
            return source.output_pin.clone();
        }

        match &source.node {
            NodeEntity::New { meta, .. } => {
                let compatible: Vec<&PinMetadata> = meta
                    .outputs
                    .iter()
                    .filter(|pin| pin.data_type != "Execution")
                    .filter(|pin| metadata_pins_are_compatible(input, pin))
                    .collect();
                match compatible.as_slice() {
                    [pin] => Some(pin.name.clone()),
                    many => many
                        .iter()
                        .find(|pin| {
                            matches!(pin.name.as_str(), "result" | "value" | "output" | "out")
                        })
                        .map(|pin| pin.name.clone())
                        .or_else(|| self.resolve_entity_output_pin(&source.node, None)),
                }
            }
            NodeEntity::Existing(_) | NodeEntity::Layer { .. } => {
                self.resolve_source_output_pin(source)
            }
        }
    }

    fn resolve_entity_output_pin(
        &self,
        entity: &NodeEntity,
        requested: Option<&str>,
    ) -> Option<String> {
        match entity {
            NodeEntity::Existing(id) => {
                let node = find_board_node(self.existing, id)?;
                if let Some(requested) = requested {
                    return find_output_pin(node, requested).map(|p| p.name.clone());
                }
                default_node_output_pin(node)
            }
            NodeEntity::New { meta, .. } => {
                if let Some(requested) = requested {
                    return metadata_output_pin(meta, requested).map(|p| p.name.clone());
                }
                default_metadata_output_pin(meta)
            }
            NodeEntity::Layer { pins, .. } => {
                if let Some(requested) = requested {
                    return pins
                        .iter()
                        .find(|pin| pin_name_matches(&pin.name, requested))
                        .map(|pin| pin.name.clone());
                }
                pins.iter()
                    .find(|pin| pin.pin_type == "Output")
                    .map(|pin| pin.name.clone())
            }
        }
    }

    fn seed_params_from_entity(&mut self, params: &[Param], entity: &NodeEntity) {
        for param in params {
            if let Some(pin) = self.resolve_entity_output_pin(entity, Some(&param.name)) {
                self.insert_symbol(
                    param.name.clone(),
                    SymbolValue::Source(ValueSource {
                        node: entity.clone(),
                        output_pin: Some(pin),
                    }),
                );
            }
        }
    }

    fn seed_top_level_variables(&mut self, ast: &BoardAst) {
        for var in &ast.variables {
            let variable_id = self.variable_id_for_decl(var);
            self.variable_refs.insert(&variable_id, &var.name);
            self.insert_symbol(var.name.clone(), SymbolValue::VariableRef { variable_id });
        }
    }

    fn variable_id_for_decl(&self, var: &VarDecl) -> String {
        if let Some(anchor) = var.anchor.as_deref() {
            return anchor.to_string();
        }

        if let Some(existing) = self
            .existing
            .variables
            .values()
            .find(|existing| existing.name == var.name)
        {
            return existing.id.clone();
        }

        for layer in self.existing.layers.values() {
            if let Some(existing) = layer
                .variables
                .values()
                .find(|existing| existing.name == var.name)
            {
                return existing.id.clone();
            }
        }

        variable_id_for_decl(var)
    }

    fn push_scope(&mut self) {
        self.symbols.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.symbols.pop();
    }

    fn insert_symbol(&mut self, name: String, source: SymbolValue) {
        if let Some(scope) = self.symbols.last_mut() {
            scope.insert(name, source);
        }
    }

    fn assign_symbol(&mut self, name: String, source: SymbolValue) {
        if let Some(scope) = self
            .symbols
            .iter_mut()
            .rev()
            .find(|scope| scope.contains_key(&name))
        {
            scope.insert(name, source);
            return;
        }
        self.insert_symbol(name, source);
    }

    fn lookup_symbol(&self, name: &str) -> Option<SymbolValue> {
        self.symbols
            .iter()
            .rev()
            .find_map(|scope| scope.get(name).cloned())
    }
}

fn ast_has_unanchored_calls(ast: &BoardAst) -> bool {
    for ev in &ast.events {
        if ev.anchor.is_none() {
            return true;
        }
        if block_has_unanchored_calls(&ev.body) {
            return true;
        }
    }
    for f in &ast.functions {
        if f.anchor.is_none() {
            return true;
        }
        if block_has_unanchored_calls(&f.body) {
            return true;
        }
    }
    false
}

fn block_has_unanchored_calls(block: &Block) -> bool {
    block.stmts.iter().any(stmt_has_unanchored_calls)
}

fn stmt_has_unanchored_calls(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Let { call, anchor, .. } | Stmt::Call { call, anchor } => {
            anchor.is_none() || call_args_have_unanchored_calls(call)
        }
        Stmt::Branch {
            call,
            condition,
            arms,
            anchor,
            ..
        } => {
            (anchor.is_none() && !is_placeholder_call(call))
                || call_args_have_unanchored_calls(call)
                || condition.as_ref().is_some_and(expr_has_unanchored_calls)
                || arms.iter().any(|arm| block_has_unanchored_calls(&arm.body))
        }
        Stmt::Loop {
            call, body, anchor, ..
        } => {
            anchor.is_none()
                || call_args_have_unanchored_calls(call)
                || block_has_unanchored_calls(body)
        }
        Stmt::Assign { value, anchor, .. }
        | Stmt::FieldAssign { value, anchor, .. }
        | Stmt::LocalAlias { value, anchor, .. } => {
            anchor.is_none() || expr_has_unanchored_calls(value)
        }
        Stmt::Handler(event) => event.anchor.is_none() || block_has_unanchored_calls(&event.body),
        Stmt::Return { values, .. } => values.iter().any(expr_has_unanchored_calls),
        Stmt::Local(_) | Stmt::Comment(_) => false,
    }
}

fn call_args_have_unanchored_calls(call: &Call) -> bool {
    call.args
        .iter()
        .any(|arg| expr_has_unanchored_calls(&arg.value))
}

fn expr_has_unanchored_calls(expr: &Expr) -> bool {
    match expr {
        Expr::Call(call) => call.anchor.is_none() || call_args_have_unanchored_calls(call),
        Expr::Field { base, .. } | Expr::Member { base, .. } => expr_has_unanchored_calls(base),
        Expr::Object(fields) => fields.iter().any(|f| expr_has_unanchored_calls(&f.value)),
        Expr::Array(items) => items.iter().any(expr_has_unanchored_calls),
        Expr::Index { base, index } => {
            expr_has_unanchored_calls(base) || expr_has_unanchored_calls(index)
        }
        Expr::Ternary {
            cond,
            then,
            otherwise,
        } => {
            expr_has_unanchored_calls(cond)
                || expr_has_unanchored_calls(then)
                || expr_has_unanchored_calls(otherwise)
        }
        Expr::Binary { lhs, rhs, .. } => {
            expr_has_unanchored_calls(lhs) || expr_has_unanchored_calls(rhs)
        }
        Expr::Ref(_) | Expr::Literal(_) => false,
    }
}

/// Synthesize the `struct_set` [`Call`] a `base.path = value` struct-field write
/// (`Stmt::FieldAssign`) expands to: `structSet({ structIn: base, field: "path", value })`.
///
/// Shared by the structural planner's `Stmt::FieldAssign` arm (which turns it into an `AddNode`)
/// and the reconcile collectors (which key it by anchor to drive config-edits and deletions) so
/// the two expansions can never drift.
fn field_assign_struct_set_call(
    base: &str,
    path: &str,
    value: &Expr,
    anchor: Option<&str>,
) -> Call {
    Call {
        node_type: String::new(),
        display: "structSet".to_string(),
        args: vec![
            Arg {
                name: "structIn".to_string(),
                value: Expr::Ref(base.to_string()),
            },
            Arg {
                name: "field".to_string(),
                value: Expr::Literal(Literal::String(path.to_string())),
            },
            Arg {
                name: "value".to_string(),
                value: value.clone(),
            },
        ],
        anchor: anchor.map(str::to_string),
    }
}

/// Materialize the synthesized `struct_set` [`Call`] for every *anchored* `Stmt::FieldAssign` in
/// `ast`, at any nesting depth. `FieldAssign` carries no `&Call`, so without this the config-edit
/// (`new_calls`) and deletion (`visible`) collectors would never see a `base.path = value` write's
/// underlying `struct_set` node. The returned owned calls back the `&Call` entries those maps key
/// by anchor. Anchorless writes are fresh (handled by the structural planner) and are skipped.
fn collect_anchored_field_assign_calls(ast: &BoardAst) -> Vec<Call> {
    let mut out = Vec::new();
    for ev in &ast.events {
        collect_field_assign_calls_in_block(&ev.body, &mut out);
    }
    for f in &ast.functions {
        collect_field_assign_calls_in_block(&f.body, &mut out);
    }
    out
}

fn collect_field_assign_calls_in_block(block: &Block, out: &mut Vec<Call>) {
    for stmt in &block.stmts {
        collect_field_assign_calls_in_stmt(stmt, out);
    }
}

fn collect_field_assign_calls_in_stmt(stmt: &Stmt, out: &mut Vec<Call>) {
    match stmt {
        Stmt::FieldAssign {
            base,
            path,
            value,
            anchor: Some(anchor),
        } => out.push(field_assign_struct_set_call(
            base,
            path,
            value,
            Some(anchor),
        )),
        Stmt::Branch { arms, .. } => {
            for arm in arms {
                collect_field_assign_calls_in_block(&arm.body, out);
            }
        }
        Stmt::Loop { body, .. } => collect_field_assign_calls_in_block(body, out),
        Stmt::Handler(event) => collect_field_assign_calls_in_block(&event.body, out),
        _ => {}
    }
}

/// Walk a `BoardAst`, recording every [`Call`] that carries an anchor, keyed by node id.
fn collect_calls<'a>(ast: &'a BoardAst, out: &mut HashMap<String, &'a Call>) {
    for ev in &ast.events {
        collect_block(&ev.body, out);
    }
    for f in &ast.functions {
        collect_block(&f.body, out);
    }
}

fn collect_block<'a>(block: &'a Block, out: &mut HashMap<String, &'a Call>) {
    for stmt in &block.stmts {
        collect_stmt(stmt, out);
    }
}

/// Walk only statement-owned calls. This is intentionally shallower than [`collect_calls`]:
/// inlined helper expressions are render conveniences, not standalone text-visible statements,
/// and must not be removed just because their anchors are absent from edited FlowScript.
fn collect_statement_calls<'a>(ast: &'a BoardAst, out: &mut HashMap<String, &'a Call>) {
    for ev in &ast.events {
        collect_statement_block(&ev.body, out);
    }
    for f in &ast.functions {
        collect_statement_block(&f.body, out);
    }
}

fn collect_statement_block<'a>(block: &'a Block, out: &mut HashMap<String, &'a Call>) {
    for stmt in &block.stmts {
        collect_statement_stmt(stmt, out);
    }
}

fn collect_statement_stmt<'a>(stmt: &'a Stmt, out: &mut HashMap<String, &'a Call>) {
    match stmt {
        Stmt::Let { call, anchor, .. } | Stmt::Call { call, anchor } => {
            collect_call_anchor_only(call, anchor.as_deref(), out)
        }
        Stmt::Branch {
            call, arms, anchor, ..
        } => {
            // Sugared boolean branches carry a placeholder call, but the statement anchor is
            // still the branch node itself — it must register as present or reconcile deletes
            // the branch node on every roundtrip.
            collect_call_anchor_only(call, anchor.as_deref(), out);
            for arm in arms {
                collect_statement_block(&arm.body, out);
            }
        }
        Stmt::Loop {
            call, body, anchor, ..
        } => {
            collect_call_anchor_only(call, anchor.as_deref(), out);
            collect_statement_block(body, out);
        }
        Stmt::Assign { value, anchor, .. } | Stmt::LocalAlias { value, anchor, .. } => {
            if let Some(anchor) = anchor.as_deref()
                && let Some((call, _)) = assigned_call_expr(value)
            {
                collect_call_anchor_only(call, Some(anchor), out);
            }
        }
        Stmt::Handler(event) => collect_statement_block(&event.body, out),
        // A `base.path = value` write's own `struct_set` node has no `&Call` to key here; its
        // anchor is registered into `visible` separately from the owned arena built by
        // `collect_anchored_field_assign_calls`, so a deleted dot-form line still flags a removal.
        Stmt::FieldAssign { .. } | Stmt::Return { .. } | Stmt::Local(_) | Stmt::Comment(_) => {}
    }
}

fn collect_call_anchor_only<'a>(
    call: &'a Call,
    fallback_anchor: Option<&str>,
    out: &mut HashMap<String, &'a Call>,
) {
    if let Some(anchor) = call.anchor.as_deref().or(fallback_anchor) {
        out.insert(anchor.to_string(), call);
    }
}

fn collect_stmt<'a>(stmt: &'a Stmt, out: &mut HashMap<String, &'a Call>) {
    match stmt {
        Stmt::Let { call, anchor, .. } | Stmt::Call { call, anchor } => {
            collect_call_with_anchor(call, anchor.as_deref(), out)
        }
        Stmt::Branch {
            call, arms, anchor, ..
        } => {
            // Collect the anchor even for placeholder-call (sugared boolean) branches — the
            // statement anchor is the branch node itself; see collect_statement_stmt. A
            // condition-form `if (cond)` round-trip keeps its control_branch node this way
            // instead of being mistaken for a deletion.
            collect_call_with_anchor(call, anchor.as_deref(), out);
            for arm in arms {
                collect_block(&arm.body, out);
            }
        }
        Stmt::Loop {
            call, body, anchor, ..
        } => {
            collect_call_with_anchor(call, anchor.as_deref(), out);
            collect_block(body, out);
        }
        Stmt::Assign { value, anchor, .. } => {
            if let Some(anchor) = anchor.as_deref()
                && collect_assigned_call_with_anchor(value, anchor, out)
            {
                return;
            }
            collect_expr(value, out);
        }
        Stmt::LocalAlias { value, anchor, .. } => {
            if let Some(anchor) = anchor.as_deref()
                && collect_assigned_call_with_anchor(value, anchor, out)
            {
                return;
            }
            collect_expr(value, out);
        }
        // A `base.path = value` write's own `struct_set` has no `&Call` to key here; its anchor is
        // registered into `new_calls` separately (see `collect_anchored_field_assign_calls`). Still
        // walk the RHS so any anchored calls feeding `value` are tracked.
        Stmt::FieldAssign { value, .. } => collect_expr(value, out),
        Stmt::Return { values, .. } => {
            for v in values {
                collect_expr(v, out);
            }
        }
        Stmt::Handler(event) => collect_block(&event.body, out),
        Stmt::Local(_) | Stmt::Comment(_) => {}
    }
}

fn collect_call<'a>(call: &'a Call, out: &mut HashMap<String, &'a Call>) {
    collect_call_with_anchor(call, None, out);
}

fn collect_call_with_anchor<'a>(
    call: &'a Call,
    fallback_anchor: Option<&str>,
    out: &mut HashMap<String, &'a Call>,
) {
    if let Some(anchor) = call.anchor.as_deref().or(fallback_anchor) {
        out.insert(anchor.to_string(), call);
    }
    for arg in &call.args {
        collect_expr(&arg.value, out);
    }
}

fn collect_expr<'a>(expr: &'a Expr, out: &mut HashMap<String, &'a Call>) {
    match expr {
        Expr::Call(call) => collect_call(call, out),
        Expr::Field { base, .. } | Expr::Member { base, .. } => collect_expr(base, out),
        Expr::Object(fields) => {
            for f in fields {
                collect_expr(&f.value, out);
            }
        }
        Expr::Array(items) => {
            for it in items {
                collect_expr(it, out);
            }
        }
        Expr::Index { base, index } => {
            collect_expr(base, out);
            collect_expr(index, out);
        }
        Expr::Ternary {
            cond,
            then,
            otherwise,
        } => {
            collect_expr(cond, out);
            collect_expr(then, out);
            collect_expr(otherwise, out);
        }
        Expr::Binary { lhs, rhs, .. } => {
            collect_expr(lhs, out);
            collect_expr(rhs, out);
        }
        Expr::Ref(_) | Expr::Literal(_) => {}
    }
}

fn collect_assigned_call_with_anchor<'a>(
    expr: &'a Expr,
    anchor: &str,
    out: &mut HashMap<String, &'a Call>,
) -> bool {
    match expr {
        Expr::Field { base, .. } => {
            if let Expr::Call(call) = base.as_ref() {
                collect_call_with_anchor(call, Some(anchor), out);
                true
            } else {
                false
            }
        }
        _ => false,
    }
}

fn assigned_call_expr(expr: &Expr) -> Option<(&Call, Option<&str>)> {
    match expr {
        Expr::Call(call) => Some((call, None)),
        Expr::Field { base, pin } => {
            let Expr::Call(call) = base.as_ref() else {
                return None;
            };
            Some((call, Some(pin.as_str())))
        }
        _ => None,
    }
}

fn is_placeholder_call(call: &Call) -> bool {
    call.node_type.is_empty() && call.display.is_empty() && call.args.is_empty()
}

fn promoted_local_aliases(block: &Block) -> HashSet<String> {
    let local_names: HashSet<String> = block
        .stmts
        .iter()
        .filter_map(|stmt| match stmt {
            Stmt::LocalAlias { name, .. } => Some(name.clone()),
            _ => None,
        })
        .collect();

    if local_names.is_empty() {
        return HashSet::new();
    }

    let mut promoted = HashSet::new();
    for stmt in &block.stmts {
        collect_nested_assignments_to(&local_names, stmt, false, &mut promoted);
    }
    promoted
}

fn collect_nested_assignments_to(
    local_names: &HashSet<String>,
    stmt: &Stmt,
    nested: bool,
    out: &mut HashSet<String>,
) {
    match stmt {
        Stmt::Assign { target, .. } if nested && local_names.contains(target) => {
            out.insert(target.clone());
        }
        Stmt::FieldAssign { base, .. } if nested && local_names.contains(base) => {
            out.insert(base.clone());
        }
        Stmt::Branch { arms, .. } => {
            for arm in arms {
                for stmt in &arm.body.stmts {
                    collect_nested_assignments_to(local_names, stmt, true, out);
                }
            }
        }
        Stmt::Loop { body, .. } => {
            for stmt in &body.stmts {
                collect_nested_assignments_to(local_names, stmt, true, out);
            }
        }
        Stmt::Handler(event) => {
            for stmt in &event.body.stmts {
                collect_nested_assignments_to(local_names, stmt, true, out);
            }
        }
        _ => {}
    }
}

fn infer_variable_types(value: Option<&flow_like_types::Value>) -> (String, String) {
    use flow_like_types::Value;
    match value {
        Some(Value::Array(items)) => {
            let data_type = items
                .iter()
                .find_map(value_data_type)
                .unwrap_or("Generic")
                .to_string();
            (data_type, "Array".to_string())
        }
        Some(value) => (
            value_data_type(value).unwrap_or("Generic").to_string(),
            "Normal".to_string(),
        ),
        None => ("Generic".to_string(), "Normal".to_string()),
    }
}

fn value_data_type(value: &flow_like_types::Value) -> Option<&'static str> {
    use flow_like_types::Value;
    match value {
        Value::String(_) => Some("String"),
        Value::Number(number) if number.is_i64() || number.is_u64() => Some("Integer"),
        Value::Number(_) => Some("Float"),
        Value::Bool(_) => Some("Boolean"),
        Value::Object(_) => Some("Struct"),
        Value::Array(_) => Some("Generic"),
        Value::Null => None,
    }
}

/// Convenience: parse FlowScript text and reconcile it against `existing` in one step.
///
/// Parse errors are surfaced as a single diagnostic with no commands, so callers can render the
/// failure without applying a partial mutation.
pub fn reconcile_text(existing: &Board, text: &str) -> ReconcileResult {
    match flow_like_ast::parse(text) {
        Ok(ast) => reconcile(existing, &ast),
        Err(err) => ReconcileResult {
            commands: Vec::new(),
            diagnostics: vec![format!(
                "FlowScript parse error at line {}, col {}: {}",
                err.line, err.col, err.message
            )],
        },
    }
}

/// Parse FlowScript text and reconcile it with catalog metadata, enabling new unanchored node
/// calls to be translated into reviewable board commands.
pub fn reconcile_text_with_catalog(
    existing: &Board,
    text: &str,
    catalog: &[NodeMetadata],
) -> ReconcileResult {
    match flow_like_ast::parse(text) {
        Ok(ast) => reconcile_with_catalog(existing, &ast, catalog),
        Err(err) => ReconcileResult {
            commands: Vec::new(),
            diagnostics: vec![format!(
                "FlowScript parse error at line {}, col {}: {}",
                err.line, err.col, err.message
            )],
        },
    }
}

/// Like [`reconcile_text_with_catalog`] but with a dynamic-pin [`MetadataEnricher`] (see
/// [`reconcile_with_catalog_enriched`]).
pub fn reconcile_text_with_catalog_enriched(
    existing: &Board,
    text: &str,
    catalog: &[NodeMetadata],
    enricher: &MetadataEnricher,
) -> ReconcileResult {
    match flow_like_ast::parse(text) {
        Ok(ast) => reconcile_with_catalog_enriched(existing, &ast, catalog, enricher),
        Err(err) => ReconcileResult {
            commands: Vec::new(),
            diagnostics: vec![format!(
                "FlowScript parse error at line {}, col {}: {}",
                err.line, err.col, err.message
            )],
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::board::{Board, ExecutionMode, ExecutionStage};
    use crate::flow::execution::LogLevel;
    use crate::flow::node::Node;
    use crate::flow::pin::ValueType;
    use crate::flow::variable::{Variable, VariableType};
    use flow_like_storage::Path;
    use std::collections::HashMap;
    use std::time::SystemTime;

    fn empty_board() -> Board {
        Board {
            id: "board".to_string(),
            name: "Board".to_string(),
            description: String::new(),
            nodes: HashMap::new(),
            variables: HashMap::new(),
            comments: HashMap::new(),
            viewport: (0.0, 0.0, 1.0),
            version: (0, 0, 1),
            stage: ExecutionStage::Dev,
            log_level: LogLevel::Info,
            execution_mode: ExecutionMode::Hybrid,
            refs: HashMap::new(),
            layers: HashMap::new(),
            page_ids: Vec::new(),
            hash: None,
            created_at: SystemTime::now(),
            updated_at: SystemTime::now(),
            parent: None,
            board_dir: Path::from("/test"),
            logic_nodes: HashMap::new(),
            app_state: None,
        }
    }

    fn connect(board: &mut Board, from_node: &str, from_pin: &str, to_node: &str, to_pin: &str) {
        crate::flow::board::commands::pins::connect_pins::connect_pins(
            board, from_node, from_pin, to_node, to_pin,
        )
        .expect("connect pins");
    }

    /// Build an `event → log` board where the log node carries a literal `text` default.
    fn board_with_log(text_default: &str) -> Board {
        let mut board = empty_board();

        let mut event = Node::new("events_simple", "Start", "", "events");
        event.id = "event".to_string();
        event.set_start(true);
        let exec_out = event
            .add_output_pin("exec_out", "Out", "", VariableType::Execution)
            .id
            .clone();
        board.nodes.insert(event.id.clone(), event);

        let mut log = Node::new("log", "Log", "", "debug");
        log.id = "log".to_string();
        let exec_in = log
            .add_input_pin("exec_in", "In", "", VariableType::Execution)
            .id
            .clone();
        log.add_output_pin("exec_out", "Out", "", VariableType::Execution);
        let text_pin = log.add_input_pin("text", "Text", "", VariableType::String);
        text_pin.default_value = Some(format!("\"{text_default}\"").into_bytes());
        board.nodes.insert(log.id.clone(), log);

        connect(&mut board, "event", &exec_out, "log", &exec_in);
        board
    }

    fn board_with_variable() -> Board {
        let mut board = empty_board();
        let mut variable = Variable::new("apiKey", VariableType::String, ValueType::Normal);
        variable.id = "var_api".to_string();
        variable.description = Some("API key".to_string());
        variable.set_default_value(flow_like_types::Value::String("old".to_string()));
        board.variables.insert(variable.id.clone(), variable);
        board
    }

    /// Finds the anchored `log` call in a lowered AST and overwrites its `text` literal.
    fn set_text_arg(ast: &mut BoardAst, value: &str) {
        for ev in &mut ast.events {
            for stmt in &mut ev.body.stmts {
                if let Stmt::Call { call, .. } = stmt
                    && call.anchor.as_deref() == Some("log")
                {
                    for arg in &mut call.args {
                        if arg.name == "text" {
                            arg.value = Expr::Literal(Literal::String(value.to_string()));
                        }
                    }
                }
            }
        }
    }

    /// Build an `event → control_branch(true) → log` board (condition-form `if` sugar).
    fn board_with_condition_branch() -> Board {
        let mut board = empty_board();

        let mut event = Node::new("events_simple", "Start", "", "events");
        event.id = "event".to_string();
        event.set_start(true);
        let event_out = event
            .add_output_pin("exec_out", "Out", "", VariableType::Execution)
            .id
            .clone();
        board.nodes.insert(event.id.clone(), event);

        let mut branch = Node::new("control_branch", "Branch", "", "control");
        branch.id = "branch".to_string();
        let branch_in = branch
            .add_input_pin("exec_in", "In", "", VariableType::Execution)
            .id
            .clone();
        let condition = branch.add_input_pin("condition", "Condition", "", VariableType::Boolean);
        condition.default_value = Some(b"true".to_vec());
        let branch_true = branch
            .add_output_pin("true", "True", "", VariableType::Execution)
            .id
            .clone();
        branch.add_output_pin("false", "False", "", VariableType::Execution);
        board.nodes.insert(branch.id.clone(), branch);

        let mut log = Node::new("log", "Log", "", "debug");
        log.id = "log".to_string();
        let log_in = log
            .add_input_pin("exec_in", "In", "", VariableType::Execution)
            .id
            .clone();
        log.add_output_pin("exec_out", "Out", "", VariableType::Execution);
        log.add_input_pin("text", "Text", "", VariableType::String);
        board.nodes.insert(log.id.clone(), log);

        connect(&mut board, "event", &event_out, "branch", &branch_in);
        connect(&mut board, "branch", &branch_true, "log", &log_in);
        board
    }

    /// Build an `event → single_choice` board whose two same-named `options` input pins
    /// hold different literal defaults (the dynamic multi-pin pattern). Returns the board
    /// plus the two option-pin ids in index order.
    fn board_with_multi_pin_node() -> (Board, String, String) {
        let mut board = empty_board();

        let mut event = Node::new("events_simple", "Start", "", "events");
        event.id = "event".to_string();
        event.set_start(true);
        let event_out = event
            .add_output_pin("exec_out", "Out", "", VariableType::Execution)
            .id
            .clone();
        board.nodes.insert(event.id.clone(), event);

        let mut choice = Node::new("single_choice", "Single Choice", "", "interaction");
        choice.id = "choice".to_string();
        let choice_in = choice
            .add_input_pin("exec_in", "In", "", VariableType::Execution)
            .id
            .clone();
        choice.add_output_pin("exec_out", "Out", "", VariableType::Execution);
        let first = choice.add_input_pin("options", "Options", "", VariableType::String);
        first.default_value = Some(b"\"a\"".to_vec());
        let first_id = first.id.clone();
        let second = choice.add_input_pin("options", "Options", "", VariableType::String);
        second.default_value = Some(b"\"b\"".to_vec());
        let second_id = second.id.clone();
        board.nodes.insert(choice.id.clone(), choice);

        connect(&mut board, "event", &event_out, "choice", &choice_in);
        (board, first_id, second_id)
    }

    fn anchored_text(board: &Board) -> String {
        super::super::board_to_flowscript(
            board,
            &flow_like_ast::RenderOptions {
                anchors: true,
                ..Default::default()
            },
        )
    }

    #[test]
    fn unchanged_multi_pin_roundtrip_emits_nothing() {
        let (board, _, _) = board_with_multi_pin_node();
        let text = anchored_text(&board);
        let result = reconcile_text(&board, &text);
        assert!(
            result.commands.is_empty(),
            "no-op multi-pin round-trip must be empty; got {:?} from text:\n{text}",
            result.commands
        );
    }

    #[test]
    fn multi_pin_edit_targets_the_right_pin_by_id() {
        let (board, _first_id, second_id) = board_with_multi_pin_node();
        let text = anchored_text(&board).replace("options: \"b\"", "options: \"c\"");
        let result = reconcile_text(&board, &text);
        let updates: Vec<_> = result
            .commands
            .iter()
            .filter_map(|c| match c {
                BoardCommand::UpdateNodePin { pin_id, value, .. } => {
                    Some((pin_id.clone(), value.clone()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            updates,
            vec![(
                second_id,
                flow_like_types::Value::String("c".to_string())
            )],
            "editing the second occurrence must update exactly the second pin, by id"
        );
    }

    #[test]
    fn unchanged_condition_branch_roundtrip_emits_no_removals() {
        // `if (cond)` sugar parses with a placeholder call; its anchor must still count as
        // present, otherwise a no-op round-trip deletes the control_branch node.
        let board = board_with_condition_branch();
        let text = super::super::board_to_flowscript(
            &board,
            &flow_like_ast::RenderOptions {
                anchors: true,
                ..Default::default()
            },
        );
        let result = reconcile_text(&board, &text);
        let removals: Vec<_> = result
            .commands
            .iter()
            .filter(|c| matches!(c, BoardCommand::RemoveNode { .. }))
            .collect();
        assert!(
            removals.is_empty(),
            "no-op round-trip must not delete nodes; got {removals:?} from text:\n{text}"
        );
    }

    #[test]
    fn literal_edit_emits_update_pin() {
        let board = board_with_log("hello");
        let mut ast = super::super::lower_to_ast(&board);
        set_text_arg(&mut ast, "world");

        let result = reconcile(&board, &ast);

        let updates: Vec<_> = result
            .commands
            .iter()
            .filter_map(|c| match c {
                BoardCommand::UpdateNodePin {
                    node_id,
                    pin_id,
                    value,
                    ..
                } => Some((node_id.as_str(), pin_id.as_str(), value.clone())),
                _ => None,
            })
            .collect();

        assert_eq!(
            updates,
            vec![(
                "log",
                "text",
                flow_like_types::Value::String("world".into())
            )],
            "changed literal should emit exactly one UpdateNodePin"
        );
    }

    #[test]
    fn unchanged_literal_emits_nothing() {
        let board = board_with_log("hello");
        let ast = super::super::lower_to_ast(&board);

        let result = reconcile(&board, &ast);

        assert!(
            result.commands.is_empty(),
            "round-tripping the board's own AST must be a no-op, got {:?}",
            result.commands
        );
    }

    #[test]
    fn unchanged_variable_roundtrip_emits_nothing() {
        let board = board_with_variable();
        let text =
            super::super::board_to_flowscript(&board, &flow_like_ast::RenderOptions::default());

        let result = reconcile_text(&board, &text);

        assert!(
            result.commands.is_empty(),
            "round-tripping variable declarations must be a no-op, got {:?}",
            result.commands
        );
    }

    #[test]
    fn new_variable_emits_create_variable() {
        let board = empty_board();
        let ast = flow_like_ast::parse("const apiKey: string = \"secret\"\n").expect("parse");

        let result = reconcile(&board, &ast);

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(result.commands.len(), 1);
        assert!(matches!(
            &result.commands[0],
            BoardCommand::CreateVariable {
                name,
                data_type,
                value_type,
                default_value: Some(value),
                ..
            } if name == "apiKey"
                && data_type == "String"
                && value_type == "Normal"
                && value == &flow_like_types::Value::String("secret".to_string())
        ));
    }

    #[test]
    fn changed_variable_default_emits_update_variable() {
        let board = board_with_variable();
        let text = super::super::board_to_flowscript(
            &board,
            &flow_like_ast::RenderOptions {
                anchors: true,
                ..Default::default()
            },
        )
        .replace("\"old\"", "\"new\"");

        let result = reconcile_text(&board, &text);

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(result.commands.len(), 1);
        assert!(matches!(
            &result.commands[0],
            BoardCommand::UpdateVariable {
                variable_id,
                default_value: Some(value),
                ..
            } if variable_id == "var_api"
                && value == &flow_like_types::Value::String("new".to_string())
        ));
    }

    #[test]
    fn removed_variable_emits_delete_variable() {
        let board = board_with_variable();
        let ast = BoardAst::default();

        let result = reconcile(&board, &ast);

        assert_eq!(result.commands.len(), 1);
        assert!(matches!(
            &result.commands[0],
            BoardCommand::RemoveVariable { variable_id, .. } if variable_id == "var_api"
        ));
    }

    #[test]
    fn interface_schema_roundtrip_emits_no_variable_update() {
        let mut board = empty_board();
        let mut variable = Variable::new("record", VariableType::Struct, ValueType::Normal);
        variable.id = "var_record".to_string();
        variable.schema = Some(
            r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","title":"Record","type":"object","properties":{"name":{"type":"string"},"tags":{"type":["array","null"],"items":{"type":"string"}}},"required":["name"]}"#
                .to_string(),
        );
        variable.set_default_value(flow_like_types::Value::Object(Default::default()));
        board.variables.insert(variable.id.clone(), variable);

        let text =
            super::super::board_to_flowscript(&board, &flow_like_ast::RenderOptions::default());
        let result = reconcile_text(&board, &text);

        assert!(
            result.commands.is_empty(),
            "interface schema text roundtrip must not rewrite the original board schema, got {:?}",
            result.commands
        );
    }

    #[test]
    fn missing_anchored_node_is_removed() {
        let board = board_with_log("hello");
        let mut ast = super::super::lower_to_ast(&board);
        // Drop the log statement from the event body to simulate a deletion in the text.
        for ev in &mut ast.events {
            ev.body.stmts.retain(
                |s| !matches!(s, Stmt::Call { call, .. } if call.anchor.as_deref() == Some("log")),
            );
        }

        let result = reconcile(&board, &ast);

        let removed: Vec<_> = result
            .commands
            .iter()
            .filter_map(|c| match c {
                BoardCommand::RemoveNode { node_id, .. } => Some(node_id.as_str()),
                _ => None,
            })
            .collect();

        assert_eq!(removed, vec!["log"], "absent text-visible node is removed");
    }

    #[test]
    fn parse_error_yields_diagnostic_no_commands() {
        let board = empty_board();
        let result = reconcile_text(&board, "this is not valid flowscript {{{");
        assert!(result.commands.is_empty());
        assert_eq!(result.diagnostics.len(), 1);
        assert!(result.diagnostics[0].contains("parse error"));
    }

    #[test]
    fn parsed_anchor_literal_edit_emits_update_pin() {
        let board = board_with_log("hello");
        let text = super::super::board_to_flowscript(
            &board,
            &flow_like_ast::RenderOptions {
                anchors: true,
                ..Default::default()
            },
        )
        .replace("\"hello\"", "\"world\"");

        let result = reconcile_text(&board, &text);

        assert_eq!(result.commands.len(), 1);
        assert!(matches!(
            &result.commands[0],
            BoardCommand::UpdateNodePin { node_id, pin_id, value, .. }
                if node_id == "log"
                    && pin_id == "text"
                    && value == &flow_like_types::Value::String("world".to_string())
        ));
    }

    fn pin_meta(name: &str, data_type: &str, _pin_type: PinType) -> PinMetadata {
        PinMetadata {
            name: name.to_string(),
            friendly_name: name.to_string(),
            description: String::new(),
            data_type: data_type.to_string(),
            value_type: "Normal".to_string(),
            default_value: None,
            schema: None,
            is_generic: false,
            valid_values: None,
            enforce_schema: false,
        }
    }

    fn pin_meta_friendly(
        name: &str,
        friendly_name: &str,
        data_type: &str,
        value_type: &str,
        pin_type: PinType,
    ) -> PinMetadata {
        let mut pin = pin_meta(name, data_type, pin_type);
        pin.friendly_name = friendly_name.to_string();
        pin.value_type = value_type.to_string();
        pin
    }

    fn catalog_meta(
        name: &str,
        friendly_name: &str,
        inputs: Vec<PinMetadata>,
        outputs: Vec<PinMetadata>,
    ) -> NodeMetadata {
        NodeMetadata {
            name: name.to_string(),
            friendly_name: friendly_name.to_string(),
            description: String::new(),
            inputs,
            outputs,
            category: None,
            required_inputs: Vec::new(),
            companion_nodes: Vec::new(),
            capability_tags: Vec::new(),
        }
    }

    /// Board: event → makeData (struct output) → log, where log.text is fed through a
    /// sugared `struct_get` (renders as `makeData.data.report_id`).
    fn board_with_struct_member_chain() -> Board {
        let mut board = empty_board();

        let mut event = Node::new("events_simple", "Start", "", "events");
        event.id = "event".to_string();
        event.set_start(true);
        let event_out = event
            .add_output_pin("exec_out", "Out", "", VariableType::Execution)
            .id
            .clone();
        board.nodes.insert(event.id.clone(), event);

        let mut producer = Node::new("make_data", "Make Data", "", "data");
        producer.id = "producer".to_string();
        let producer_in = producer
            .add_input_pin("exec_in", "In", "", VariableType::Execution)
            .id
            .clone();
        let producer_out = producer
            .add_output_pin("exec_out", "Out", "", VariableType::Execution)
            .id
            .clone();
        let data_out = producer
            .add_output_pin("data", "Data", "", VariableType::Struct)
            .id
            .clone();
        board.nodes.insert(producer.id.clone(), producer);

        let mut getter = Node::new("struct_get", "Get Field", "", "structs");
        getter.id = "getter".to_string();
        let struct_in = getter
            .add_input_pin("struct", "Struct", "", VariableType::Struct)
            .id
            .clone();
        let field_pin = getter.add_input_pin("field", "Field", "", VariableType::String);
        field_pin.default_value = Some(b"\"report_id\"".to_vec());
        let value_out = getter
            .add_output_pin("value", "Value", "", VariableType::Generic)
            .id
            .clone();
        board.nodes.insert(getter.id.clone(), getter);

        let mut log = Node::new("log", "Log", "", "debug");
        log.id = "log".to_string();
        let log_in = log
            .add_input_pin("exec_in", "In", "", VariableType::Execution)
            .id
            .clone();
        log.add_output_pin("exec_out", "Out", "", VariableType::Execution);
        let text_in = log
            .add_input_pin("text", "Text", "", VariableType::String)
            .id
            .clone();
        board.nodes.insert(log.id.clone(), log);

        connect(&mut board, "event", &event_out, "producer", &producer_in);
        connect(&mut board, "producer", &producer_out, "log", &log_in);
        connect(&mut board, "producer", &data_out, "getter", &struct_in);
        connect(&mut board, "getter", &value_out, "log", &text_in);
        board
    }

    fn member_chain_catalog() -> Vec<NodeMetadata> {
        vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "make_data",
                "Make Data",
                vec![pin_meta("exec_in", "Execution", PinType::Input)],
                vec![
                    pin_meta("exec_out", "Execution", PinType::Output),
                    pin_meta("data", "Struct", PinType::Output),
                ],
            ),
            catalog_meta(
                "struct_get",
                "Get Field",
                vec![
                    pin_meta("struct", "Struct", PinType::Input),
                    pin_meta("field", "String", PinType::Input),
                ],
                vec![pin_meta("value", "Generic", PinType::Output)],
            ),
            catalog_meta(
                "log",
                "Log",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("text", "String", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
        ]
    }

    #[test]
    fn unchanged_struct_member_roundtrip_reuses_accessor() {
        // A sugared struct_get (`base.field` in text) must reuse the existing accessor on a
        // no-op apply — not rewire the consumer to the bare base value or add a duplicate.
        let board = board_with_struct_member_chain();
        let text = anchored_text(&board);
        let result = reconcile_text_with_catalog(&board, &text, &member_chain_catalog());
        assert!(
            result.commands.is_empty(),
            "no-op member-chain round-trip must be empty; got {:?} from text:\n{text}",
            result.commands
        );
    }

    #[test]
    fn unchanged_variable_ref_roundtrip_reuses_variable_get() {
        // A bare variable ref in text lowers from a variable_get node; a no-op apply must
        // reuse that node instead of materializing a duplicate reader.
        let mut board = empty_board();
        let mut variable = Variable::new("greeting", VariableType::String, ValueType::Normal);
        variable.id = "var1".to_string();
        variable.set_default_value(flow_like_types::Value::String("hi".to_string()));
        board.variables.insert(variable.id.clone(), variable);

        let mut event = Node::new("events_simple", "Start", "", "events");
        event.id = "event".to_string();
        event.set_start(true);
        let event_out = event
            .add_output_pin("exec_out", "Out", "", VariableType::Execution)
            .id
            .clone();
        board.nodes.insert(event.id.clone(), event);

        let mut reader = Node::new("variable_get", "Get Variable", "", "variables");
        reader.id = "reader".to_string();
        let var_ref = reader.add_input_pin("var_ref", "Variable", "", VariableType::String);
        var_ref.default_value = Some(b"\"var1\"".to_vec());
        let value_out = reader
            .add_output_pin("value_ref", "Value", "", VariableType::Generic)
            .id
            .clone();
        board.nodes.insert(reader.id.clone(), reader);

        let mut log = Node::new("log", "Log", "", "debug");
        log.id = "log".to_string();
        let log_in = log
            .add_input_pin("exec_in", "In", "", VariableType::Execution)
            .id
            .clone();
        log.add_output_pin("exec_out", "Out", "", VariableType::Execution);
        let text_in = log
            .add_input_pin("text", "Text", "", VariableType::String)
            .id
            .clone();
        board.nodes.insert(log.id.clone(), log);

        connect(&mut board, "event", &event_out, "log", &log_in);
        connect(&mut board, "reader", &value_out, "log", &text_in);

        let text = anchored_text(&board);
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "variable_get",
                "Get Variable",
                vec![pin_meta("var_ref", "String", PinType::Input)],
                vec![pin_meta("value_ref", "Generic", PinType::Output)],
            ),
            catalog_meta(
                "log",
                "Log",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("text", "String", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
        ];
        let result = reconcile_text_with_catalog(&board, &text, &catalog);
        assert!(
            result.commands.is_empty(),
            "no-op variable-ref round-trip must be empty; got {:?} from text:\n{text}",
            result.commands
        );
    }

    #[test]
    fn inserting_before_fan_in_only_splices_own_chain() {
        // Two events converge on one log node (exec fan-in). Inserting a new node into one
        // chain must splice only that chain's edge — the other event's wiring stays intact.
        let mut board = empty_board();

        for event_id in ["event_a", "event_b"] {
            let mut event = Node::new(
                if event_id == "event_a" {
                    "event_start"
                } else {
                    "event_timer"
                },
                "Event",
                "",
                "events",
            );
            event.id = event_id.to_string();
            event.set_start(true);
            let exec_out = event
                .add_output_pin("exec_out", "Out", "", VariableType::Execution)
                .id
                .clone();
            board.nodes.insert(event.id.clone(), event);
            let _ = exec_out;
        }

        let mut log = Node::new("log", "Log", "", "debug");
        log.id = "log".to_string();
        let log_in = log
            .add_input_pin("exec_in", "In", "", VariableType::Execution)
            .id
            .clone();
        log.add_output_pin("exec_out", "Out", "", VariableType::Execution);
        let text_pin = log.add_input_pin("text", "Text", "", VariableType::String);
        text_pin.default_value = Some(b"\"hi\"".to_vec());
        board.nodes.insert(log.id.clone(), log);

        for event_id in ["event_a", "event_b"] {
            let exec_out = board.nodes[event_id]
                .pins
                .values()
                .find(|p| p.pin_type == PinType::Output)
                .map(|p| p.id.clone())
                .expect("event exec out");
            connect(&mut board, event_id, &exec_out, "log", &log_in);
        }

        let text = anchored_text(&board);
        let log_line = text
            .lines()
            .find(|line| line.contains("//@n:log"))
            .expect("log statement in rendered text")
            .to_string();
        let edited = text.replace(&log_line, &format!("    notify()\n{log_line}"));

        let catalog = vec![
            catalog_meta(
                "event_start",
                "Event Start",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "event_timer",
                "Event Timer",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "notify",
                "Notify",
                vec![pin_meta("exec_in", "Execution", PinType::Input)],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "log",
                "Log",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("text", "String", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
        ];
        let result = reconcile_text_with_catalog(&board, &edited, &catalog);

        let disconnects: Vec<_> = result
            .commands
            .iter()
            .filter_map(|c| match c {
                BoardCommand::DisconnectPins {
                    from_node, to_node, ..
                } => Some((from_node.clone(), to_node.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(
            disconnects.len(),
            1,
            "only the edited chain's edge may be spliced; got {disconnects:?} from:\n{edited}"
        );
        assert_eq!(disconnects[0].1, "log");
        assert!(
            disconnects[0].0.starts_with("event_"),
            "splice must originate from the chain's own event; got {disconnects:?}"
        );
    }

    #[test]
    fn catalog_aware_reconcile_adds_unanchored_calls() {
        let board = empty_board();
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "log",
                "Log",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("text", "String", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
        ];

        let result = reconcile_text_with_catalog(
            &board,
            r#"eventsSimple() {
    log({ text: "hello" })
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(result.commands.len(), 4);
        assert!(matches!(
            &result.commands[0],
            BoardCommand::AddNode { node_type, ref_id, .. }
                if node_type == "events_simple" && ref_id.as_deref() == Some("$0")
        ));
        assert!(matches!(
            &result.commands[1],
            BoardCommand::AddNode { node_type, ref_id, .. }
                if node_type == "log" && ref_id.as_deref() == Some("$1")
        ));
        assert!(matches!(
            &result.commands[2],
            BoardCommand::UpdateNodePin { node_id, pin_id, value, .. }
                if node_id == "$1"
                    && pin_id == "text"
                    && value == &flow_like_types::Value::String("hello".to_string())
        ));
        assert!(matches!(
            &result.commands[3],
            BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                if from_node == "$0"
                    && from_pin == "exec_out"
                    && to_node == "$1"
                    && to_pin == "exec_in"
        ));
    }

    #[test]
    fn catalog_aware_reconcile_skips_synthetic_tools_arg_without_diagnostic() {
        // `agentRegisterFunctionTools` has no `tools` input pin — its function references are a
        // synthetic `tools:` argument the decompiler emits. Reconcile must skip it rather than
        // report a missing pin (which surfaced to users as a false "FlowScript apply blocked").
        let board = empty_board();
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "agent_from_model",
                "Agent From Model",
                Vec::new(),
                vec![pin_meta("agent_out", "Struct", PinType::Output)],
            ),
            catalog_meta(
                "agent_register_function_tools",
                "Register Function Tools",
                vec![pin_meta("agent_in", "Struct", PinType::Input)],
                vec![pin_meta("agent_out", "Struct", PinType::Output)],
            ),
        ];

        let result = reconcile_text_with_catalog(
            &board,
            r#"eventsSimple() {
    const agent = agentRegisterFunctionTools({ agentIn: agentFromModel({}), tools: [fetchPage] })
}
"#,
            &catalog,
        );

        assert!(
            !result
                .diagnostics
                .iter()
                .any(|d| d.contains("no input pin named `tools`")),
            "synthetic tools arg must not produce a missing-pin diagnostic: {:?}",
            result.diagnostics
        );
        assert!(
            result.commands.iter().any(|command| matches!(
                command,
                BoardCommand::AddNode { node_type, .. }
                    if node_type == "agent_register_function_tools"
            )),
            "agent node should still be added: {:?}",
            result.commands
        );
        // The synthetic `tools:` arg on a NEW node materializes as a SetNodeFunctionRefs command
        // carrying the referenced target names for the applier to resolve.
        assert!(
            result.commands.iter().any(|command| matches!(
                command,
                BoardCommand::SetNodeFunctionRefs { fn_refs, .. }
                    if fn_refs == &vec!["fetchPage".to_string()]
            )),
            "expected SetNodeFunctionRefs with [fetchPage]: {:?}",
            result.commands
        );
    }

    #[test]
    fn catalog_aware_reconcile_uses_metadata_enricher_for_dynamic_pins() {
        let board = empty_board();
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            // Static metadata only knows the `shape` driver pin; a/b inputs and the `out` output are
            // "dynamic" and supplied by the enricher based on the shape literal.
            catalog_meta(
                "dynamic_node",
                "Dynamic Node",
                vec![pin_meta("shape", "String", PinType::Input)],
                Vec::new(),
            ),
            catalog_meta(
                "text_source",
                "Text Source",
                Vec::new(),
                vec![pin_meta("text", "String", PinType::Output)],
            ),
            catalog_meta(
                "sink",
                "Sink",
                vec![pin_meta("input", "Generic", PinType::Input)],
                Vec::new(),
            ),
        ];

        let enricher: MetadataEnricher = Box::new(
            |meta: &NodeMetadata,
             args: &[(String, flow_like_types::Value)],
             _board: &Board|
             -> Option<NodeMetadata> {
                if meta.name != "dynamic_node" {
                    return None;
                }
                let shape = args
                    .iter()
                    .find(|(name, _)| name == "shape")
                    .and_then(|(_, value)| value.as_str());
                if shape != Some("ab") {
                    return None;
                }
                let mut enriched = meta.clone();
                enriched
                    .inputs
                    .push(pin_meta("a", "Generic", PinType::Input));
                enriched
                    .inputs
                    .push(pin_meta("b", "Generic", PinType::Input));
                enriched
                    .outputs
                    .push(pin_meta("out", "Generic", PinType::Output));
                Some(enriched)
            },
        );

        let result = reconcile_text_with_catalog_enriched(
            &board,
            r#"eventsSimple() {
    const d = dynamicNode({ shape: "ab", a: textSource().text, b: textSource().text })
    sink({ input: d.out })
}
"#,
            &catalog,
            &enricher,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        // Enricher-provided INPUT pins a and b are recognized and wired.
        for placeholder in ["a", "b"] {
            assert!(
                result.commands.iter().any(|command| {
                    matches!(
                        command,
                        BoardCommand::ConnectPins { from_node, to_node, to_pin, .. }
                            if command_node_type(&result.commands, from_node).as_deref()
                                == Some("text_source")
                                && command_node_type(&result.commands, to_node).as_deref()
                                    == Some("dynamic_node")
                                && to_pin == placeholder
                    )
                }),
                "dynamic input `{placeholder}` not wired; commands: {:?}",
                result.commands
            );
        }
        // Enricher-provided OUTPUT pin `out` resolves for `d.out` and wires into the sink.
        assert!(
            result.commands.iter().any(|command| {
                matches!(
                    command,
                    BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                        if command_node_type(&result.commands, from_node).as_deref()
                            == Some("dynamic_node")
                            && from_pin == "out"
                            && command_node_type(&result.commands, to_node).as_deref() == Some("sink")
                            && to_pin == "input"
                )
            }),
            "dynamic output `out` not wired into sink; commands: {:?}",
            result.commands
        );
    }

    #[test]
    fn catalog_aware_reconcile_lowers_event_return_to_result_node() {
        // A `return` inside a keyword-less event/tool entry must NOT be rejected — it reverses the
        // `events_generic_return_result` sugar so agent tools and event handlers can return a value.
        let board = empty_board();
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "events_generic_return_result",
                "Return Generic Result",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("response", "Generic", PinType::Input),
                ],
                Vec::new(),
            ),
        ];

        let result = reconcile_text_with_catalog(
            &board,
            r#"eventsSimple() {
    return "done"
}
"#,
            &catalog,
        );

        assert!(
            !result
                .diagnostics
                .iter()
                .any(|d| d.contains("only supported inside FlowScript functions")),
            "event return must not be rejected: {:?}",
            result.diagnostics
        );
        assert!(
            result.commands.iter().any(|command| matches!(
                command,
                BoardCommand::AddNode { node_type, .. }
                    if node_type == "events_generic_return_result"
            )),
            "return should add a result node: {:?}",
            result.commands
        );
        assert!(
            result.commands.iter().any(|command| matches!(
                command,
                BoardCommand::UpdateNodePin { pin_id, value, .. }
                    if pin_id == "response"
                        && value == &flow_like_types::Value::String("done".to_string())
            )),
            "return value should feed the response pin: {:?}",
            result.commands
        );
    }

    #[test]
    fn catalog_aware_reconcile_chains_exec_through_and_after_loops() {
        let board = empty_board();
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "for_each",
                "For Each",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("array", "Generic", PinType::Input),
                ],
                vec![
                    pin_meta("loop", "Execution", PinType::Output),
                    pin_meta("done", "Execution", PinType::Output),
                    pin_meta("item", "Generic", PinType::Output),
                ],
            ),
            catalog_meta(
                "log",
                "Log",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("text", "String", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
        ];

        let result = reconcile_text_with_catalog(
            &board,
            r#"eventsSimple() {
    for (const item of forEach()) {
        log({ text: "inner" })
    }
    log({ text: "after" })
    log({ text: "last" })
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);

        let connects: Vec<(String, String, String, String)> = result
            .commands
            .iter()
            .filter_map(|command| match command {
                BoardCommand::ConnectPins {
                    from_node,
                    from_pin,
                    to_node,
                    to_pin,
                    ..
                } => Some((
                    from_node.clone(),
                    from_pin.clone(),
                    to_node.clone(),
                    to_pin.clone(),
                )),
                _ => None,
            })
            .collect();

        // $0 event, $1 for_each, $2 log(inner), $3 log(after), $4 log(last)
        let expected = [
            ("$0", "exec_out", "$1", "exec_in"),
            ("$1", "loop", "$2", "exec_in"),
            ("$1", "done", "$3", "exec_in"),
            // The regression: a statement AFTER a loop must chain onward from ITS exec output.
            // Gating the cursor advance on the cursor's own (non-default) pin instead of the
            // streaming-preferred output left every statement after the loop dangling.
            ("$3", "exec_out", "$4", "exec_in"),
        ];
        for (from_node, from_pin, to_node, to_pin) in expected {
            assert!(
                connects.iter().any(|(fnode, fpin, tnode, tpin)| {
                    fnode == from_node && fpin == from_pin && tnode == to_node && tpin == to_pin
                }),
                "missing exec connection {from_node}.{from_pin} -> {to_node}.{to_pin}; got {connects:?}"
            );
        }
        assert!(
            !connects
                .iter()
                .any(|(fnode, fpin, tnode, _)| fnode == "$1" && fpin == "done" && tnode == "$4"),
            "log(last) must chain from log(after), not from the loop's done pin again; got {connects:?}"
        );
    }

    #[test]
    fn catalog_aware_reconcile_splices_impure_argument_calls_into_exec_chain() {
        let board = empty_board();
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            // Pure: no exec pins — must stay data-only.
            catalog_meta(
                "make_struct",
                "Make Struct",
                Vec::new(),
                vec![pin_meta("struct", "Struct", PinType::Output)],
            ),
            // Impure: has exec pins — must be spliced into the exec chain.
            catalog_meta(
                "faker_full_name",
                "Fake Full Name",
                vec![pin_meta("exec_in", "Execution", PinType::Input)],
                vec![
                    pin_meta("exec_out", "Execution", PinType::Output),
                    pin_meta("full_name", "String", PinType::Output),
                ],
            ),
            catalog_meta(
                "set_field",
                "Set Field",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("struct", "Struct", PinType::Input),
                    pin_meta("value", "String", PinType::Input),
                ],
                vec![
                    pin_meta("exec_out", "Execution", PinType::Output),
                    pin_meta("result", "Struct", PinType::Output),
                ],
            ),
            catalog_meta(
                "log",
                "Log",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("text", "String", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
        ];

        let result = reconcile_text_with_catalog(
            &board,
            r#"eventsSimple() {
    setField({ struct: makeStruct(), value: fakerFullName() })
    log({ text: "done" })
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);

        let connects: Vec<(String, String, String, String)> = result
            .commands
            .iter()
            .filter_map(|command| match command {
                BoardCommand::ConnectPins {
                    from_node,
                    from_pin,
                    to_node,
                    to_pin,
                    ..
                } => Some((
                    from_node.clone(),
                    from_pin.clone(),
                    to_node.clone(),
                    to_pin.clone(),
                )),
                _ => None,
            })
            .collect();

        // $0 event, $1 set_field, $2 make_struct (pure), $3 faker (impure), $4 log
        let expected = [
            // The impure argument call is spliced into the chain ahead of its consumer…
            ("$0", "exec_out", "$3", "exec_in"),
            ("$3", "exec_out", "$1", "exec_in"),
            // …and the chain continues normally after the consuming statement.
            ("$1", "exec_out", "$4", "exec_in"),
            // Data wiring stays intact.
            ("$3", "full_name", "$1", "value"),
            ("$2", "struct", "$1", "struct"),
        ];
        for (from_node, from_pin, to_node, to_pin) in expected {
            assert!(
                connects.iter().any(|(fnode, fpin, tnode, tpin)| {
                    fnode == from_node && fpin == from_pin && tnode == to_node && tpin == to_pin
                }),
                "missing connection {from_node}.{from_pin} -> {to_node}.{to_pin}; got {connects:?}"
            );
        }
        // The pure helper must not receive exec wiring.
        assert!(
            !connects.iter().any(|(fnode, fpin, tnode, tpin)| {
                (fnode == "$2" && fpin.contains("exec")) || (tnode == "$2" && tpin.contains("exec"))
            }),
            "make_struct is pure and must stay data-only; got {connects:?}"
        );
    }

    #[test]
    fn catalog_aware_reconcile_resolves_local_literal_aliases() {
        let board = empty_board();
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "log",
                "Log",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("text", "String", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
        ];

        let result = reconcile_text_with_catalog(
            &board,
            r#"eventsSimple() {
    let text = "hello"
    log({ text: text })
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::UpdateNodePin { node_id, pin_id, value, .. }
                    if node_id == "$1"
                        && pin_id == "text"
                        && value == &flow_like_types::Value::String("hello".to_string())
            )
        }));
    }

    #[test]
    fn catalog_aware_reconcile_creates_function_layers_from_functions() {
        let board = empty_board();
        let catalog = vec![catalog_meta(
            "string_format",
            "String Format",
            vec![
                pin_meta("exec_in", "Execution", PinType::Input),
                pin_meta("format_string", "String", PinType::Input),
                pin_meta("name", "String", PinType::Input),
            ],
            vec![
                pin_meta("exec_out", "Execution", PinType::Output),
                pin_meta("value", "String", PinType::Output),
            ],
        )];

        let result = reconcile_text_with_catalog(
            &board,
            r#"function greet(name: string): (message: string) {
    const formatted = stringFormat({ formatString: "Hello {name}", name: name })
    return formatted.value
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(matches!(
            &result.commands[0],
            BoardCommand::CreateLayer {
                name,
                ref_id,
                layer_type,
                pins: Some(pins),
                ..
            } if name == "greet"
                && ref_id.as_deref() == Some("$0")
                && layer_type.as_deref() == Some("Function")
                && pins.iter().any(|pin| pin.name == "name" && pin.pin_type == "Input")
                && pins.iter().any(|pin| pin.name == "message" && pin.pin_type == "Output")
        ));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::AddNode { node_type, target_layer, .. }
                    if node_type == "string_format" && target_layer.as_deref() == Some("$0")
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if from_node == "$0"
                        && from_pin == "name"
                        && to_node == "$1"
                        && to_pin == "name"
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if from_node == "$1"
                        && from_pin == "value"
                        && to_node == "$0"
                        && to_pin == "message"
            )
        }));
    }

    #[test]
    fn impure_function_gets_exec_boundary_pins_and_body_wiring() {
        let board = empty_board();
        let catalog = vec![catalog_meta(
            "log",
            "Log",
            vec![
                pin_meta("exec_in", "Execution", PinType::Input),
                pin_meta("message", "String", PinType::Input),
            ],
            vec![pin_meta("exec_out", "Execution", PinType::Output)],
        )];

        let result = reconcile_text_with_catalog(
            &board,
            r#"function notify(message: string) {
    log({ message: message })
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(matches!(
            &result.commands[0],
            BoardCommand::CreateLayer { name, pins: Some(pins), .. }
                if name == "notify"
                    && pins.iter().any(|pin| pin.name == "exec_in"
                        && pin.pin_type == "Input"
                        && pin.data_type == "Execution")
                    && pins.iter().any(|pin| pin.name == "exec_out"
                        && pin.pin_type == "Output"
                        && pin.data_type == "Execution")
        ));
        // Body entry: layer exec_in → log exec_in; exit: log exec_out → layer exec_out.
        assert!(result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                if from_node == "$0" && from_pin == "exec_in" && to_node == "$1" && to_pin == "exec_in"
        )));
        assert!(result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                if from_node == "$1" && from_pin == "exec_out" && to_node == "$0" && to_pin == "exec_out"
        )));
    }

    #[test]
    fn pure_function_layer_stays_free_of_exec_pins() {
        let board = empty_board();
        let result = reconcile_text_with_catalog(
            &board,
            r#"function greet(name: string): (message: string) {
    const formatted = stringFormat({ formatString: "Hello {name}", name: name })
    return formatted.value
}
"#,
            &string_format_dynamic_catalog(),
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(matches!(
            &result.commands[0],
            BoardCommand::CreateLayer { pins: Some(pins), .. }
                if pins.iter().all(|pin| pin.data_type != "Execution")
        ));
    }

    #[test]
    fn calling_a_declared_function_creates_call_function_node() {
        let board = empty_board();
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                vec![],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "control_call_function",
                "Call Function",
                vec![pin_meta("function_layer_id", "String", PinType::Input)],
                vec![],
            ),
            catalog_meta(
                "log",
                "Log",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("message", "String", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
        ];

        let result = reconcile_text_with_catalog(
            &board,
            r#"run() {
    notify({ message: "hi" })
}

function notify(message: string) {
    log({ message: message })
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        // Layer $0 (prepared first), event $1, call node $2, body log node $3.
        assert!(result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::AddNode { node_type, ref_id, target_layer, .. }
                if node_type == "control_call_function"
                    && ref_id.as_deref() == Some("$2")
                    && target_layer.is_none()
        )));
        // The call node targets the new function layer by its ref.
        assert!(result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::UpdateNodePin { node_id, pin_id, value, .. }
                if node_id == "$2"
                    && pin_id == "function_layer_id"
                    && value == &flow_like_types::Value::String("$0".to_string())
        )));
        // The call's literal argument lands on the mirrored `message` pin.
        assert!(result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::UpdateNodePin { node_id, pin_id, value, .. }
                if node_id == "$2"
                    && pin_id == "message"
                    && value == &flow_like_types::Value::String("hi".to_string())
        )));
        // The call node joins the event's execution chain.
        assert!(result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                if from_node == "$1" && from_pin == "exec_out" && to_node == "$2" && to_pin == "exec_in"
        )));
        // And the function body chains from the layer's exec boundary.
        assert!(result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                if from_node == "$0" && from_pin == "exec_in" && to_node == "$3" && to_pin == "exec_in"
        )));
    }

    #[test]
    fn new_if_else_sugar_synthesizes_control_branch_with_labeled_arms() {
        let board = empty_board();
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                vec![],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "control_branch",
                "Branch",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("condition", "Boolean", PinType::Input),
                ],
                vec![
                    pin_meta("true", "Execution", PinType::Output),
                    pin_meta("false", "Execution", PinType::Output),
                ],
            ),
            catalog_meta(
                "log",
                "Log",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("message", "String", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
        ];

        let result = reconcile_text_with_catalog(
            &board,
            r#"run() {
    if (true) {
        log({ message: "yes" })
    } else {
        log({ message: "no" })
    }
    log({ message: "after" })
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        // Event $0, branch $1, then-arm log $2, else-arm log $3, after log $4.
        assert!(result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::AddNode { node_type, ref_id, .. }
                if node_type == "control_branch" && ref_id.as_deref() == Some("$1")
        )));
        assert!(result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::UpdateNodePin { node_id, pin_id, value, .. }
                if node_id == "$1"
                    && pin_id == "condition"
                    && value == &flow_like_types::Value::Bool(true)
        )));
        // Each arm wires from ITS labeled exec pin.
        assert!(result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                if from_node == "$1" && from_pin == "true" && to_node == "$2" && to_pin == "exec_in"
        )));
        assert!(result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                if from_node == "$1" && from_pin == "false" && to_node == "$3" && to_pin == "exec_in"
        )));
        // Fully-armed branch: the statement after the if/else must NOT steal an arm pin.
        assert!(!result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::ConnectPins { from_node, to_node, .. }
                if from_node == "$1" && to_node == "$4"
        )));
    }

    #[test]
    fn lone_if_continues_following_statement_from_false_pin() {
        let board = empty_board();
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                vec![],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "control_branch",
                "Branch",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("condition", "Boolean", PinType::Input),
                ],
                vec![
                    pin_meta("true", "Execution", PinType::Output),
                    pin_meta("false", "Execution", PinType::Output),
                ],
            ),
            catalog_meta(
                "log",
                "Log",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("message", "String", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
        ];

        let result = reconcile_text_with_catalog(
            &board,
            r#"run() {
    if (true) {
        log({ message: "yes" })
    }
    log({ message: "after" })
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        // Event $0, branch $1, then-arm log $2, after log $3: the statement after a lone `if`
        // continues from the unclaimed `false` pin.
        assert!(result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                if from_node == "$1" && from_pin == "false" && to_node == "$3" && to_pin == "exec_in"
        )));
    }

    /// Board: event → log_mid → log_tail (execution chain with anchored nodes).
    fn board_with_exec_chain() -> Board {
        let mut board = empty_board();

        let mut event = Node::new("events_simple", "Start", "", "events");
        event.id = "event".to_string();
        event.set_start(true);
        let event_out = event
            .add_output_pin("exec_out", "Out", "", VariableType::Execution)
            .id
            .clone();
        board.nodes.insert(event.id.clone(), event);

        let mut mid = Node::new("log", "Log", "", "debug");
        mid.id = "log_mid".to_string();
        let mid_in = mid
            .add_input_pin("exec_in", "In", "", VariableType::Execution)
            .id
            .clone();
        let mid_out = mid
            .add_output_pin("exec_out", "Out", "", VariableType::Execution)
            .id
            .clone();
        mid.add_input_pin("text", "Text", "", VariableType::String)
            .default_value = Some(b"\"mid\"".to_vec());
        board.nodes.insert(mid.id.clone(), mid);

        let mut tail = Node::new("log", "Log", "", "debug");
        tail.id = "log_tail".to_string();
        let tail_in = tail
            .add_input_pin("exec_in", "In", "", VariableType::Execution)
            .id
            .clone();
        tail.add_output_pin("exec_out", "Out", "", VariableType::Execution);
        tail.add_input_pin("text", "Text", "", VariableType::String)
            .default_value = Some(b"\"tail\"".to_vec());
        board.nodes.insert(tail.id.clone(), tail);

        connect(&mut board, "event", &event_out, "log_mid", &mid_in);
        connect(&mut board, "log_mid", &mid_out, "log_tail", &tail_in);
        board
    }

    fn exec_chain_catalog() -> Vec<NodeMetadata> {
        vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                vec![],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "log",
                "Log",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("text", "String", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
        ]
    }

    #[test]
    fn deleting_mid_chain_statement_bridges_execution() {
        let board = board_with_exec_chain();
        let text = crate::flow::ast::board_to_flowscript(
            &board,
            &crate::flow::ast::RenderOptions {
                anchors: true,
                ..Default::default()
            },
        );
        let edited: String = text
            .lines()
            .filter(|line| !line.contains("//@n:log_mid"))
            .collect::<Vec<_>>()
            .join("\n");

        let result = reconcile_text_with_catalog(&board, &edited, &exec_chain_catalog());
        assert!(result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::RemoveNode { node_id, .. } if node_id == "log_mid"
        )));
        assert!(
            result.commands.iter().any(|command| matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if from_node == "event"
                        && from_pin == "exec_out"
                        && to_node == "log_tail"
                        && to_pin == "exec_in"
            )),
            "deletion must re-join the severed chain: {:?}",
            result.commands
        );
    }

    #[test]
    fn replacing_mid_chain_statement_does_not_bridge_over_the_replacement() {
        let board = board_with_exec_chain();
        let text = crate::flow::ast::board_to_flowscript(
            &board,
            &crate::flow::ast::RenderOptions {
                anchors: true,
                ..Default::default()
            },
        );
        let edited: String = text
            .lines()
            .map(|line| {
                if line.contains("//@n:log_mid") {
                    "    log({ text: \"replacement\" })".to_string()
                } else {
                    line.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");

        let result = reconcile_text_with_catalog(&board, &edited, &exec_chain_catalog());
        // The replacement node takes over the chain...
        assert!(result.commands.iter().any(|command| matches!(
            command,
            BoardCommand::ConnectPins { from_node, from_pin, to_node, .. }
                if from_node == "event" && from_pin == "exec_out" && to_node.starts_with('$')
        )));
        // ...so no bridge may steal the event's exec output back to the old successor
        // (exec outputs are single-target: a later bridge would orphan the replacement).
        assert!(
            !result.commands.iter().any(|command| matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, .. }
                    if from_node == "event" && from_pin == "exec_out" && to_node == "log_tail"
            )),
            "bridge must skip sources the plan already drives: {:?}",
            result.commands
        );
    }

    #[test]
    fn layer_node_limit_rejects_oversized_edit() {
        let board = empty_board();
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                vec![],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "log",
                "Log",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("message", "String", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
        ];

        let mut flowscript = String::from("run() {\n");
        for index in 0..MAX_NODES_PER_LAYER {
            flowscript.push_str(&format!("    log({{ message: \"{index}\" }})\n"));
        }
        flowscript.push_str("}\n");

        let result = reconcile_text_with_catalog(&board, &flowscript, &catalog);
        assert!(
            result.commands.is_empty(),
            "oversized edit must queue nothing"
        );
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.contains("max 50")),
            "{:?}",
            result.diagnostics
        );
    }

    /// Catalog for a `string_format` node exactly as its `get_node()` declares it: a `format_string`
    /// input and a `value` output, with NO placeholder pins (those are minted by `on_update`).
    fn string_format_dynamic_catalog() -> Vec<NodeMetadata> {
        vec![catalog_meta(
            "string_format",
            "String Format",
            vec![pin_meta("format_string", "String", PinType::Input)],
            vec![pin_meta("value", "String", PinType::Output)],
        )]
    }

    #[test]
    fn string_format_wired_placeholder_pin_connects_without_diagnostic() {
        let board = empty_board();
        let result = reconcile_text_with_catalog(
            &board,
            r#"function greet(name: string): (message: string) {
    const formatted = stringFormat({ formatString: "Hello {name}", name: name })
    return formatted.value
}
"#,
            &string_format_dynamic_catalog(),
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(
            result.commands.iter().any(|command| matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if from_node == "$0"
                        && from_pin == "name"
                        && to_node == "$1"
                        && to_pin == "name"
            )),
            "placeholder `{{name}}` should wire the function param into the synthesized `name` pin: {:?}",
            result.commands
        );
    }

    #[test]
    fn string_format_literal_placeholder_pin_emits_update_without_diagnostic() {
        let board = empty_board();
        let result = reconcile_text_with_catalog(
            &board,
            r#"function greet(): (message: string) {
    const formatted = stringFormat({ formatString: "Count {idx}", idx: "five" })
    return formatted.value
}
"#,
            &string_format_dynamic_catalog(),
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(
            result.commands.iter().any(|command| matches!(
                command,
                BoardCommand::UpdateNodePin { pin_id, value, .. }
                    if pin_id == "idx"
                        && value == &flow_like_types::Value::String("five".to_string())
            )),
            "literal placeholder value should set the synthesized `idx` pin: {:?}",
            result.commands
        );
    }

    #[test]
    fn string_format_non_placeholder_arg_still_diagnoses() {
        let board = empty_board();
        let result = reconcile_text_with_catalog(
            &board,
            r#"function greet(): (message: string) {
    const formatted = stringFormat({ formatString: "Count {idx}", jdx: "x" })
    return formatted.value
}
"#,
            &string_format_dynamic_catalog(),
        );

        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.contains("no input pin named `jdx`")),
            "an arg that is not a template placeholder must not be synthesized: {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn config_edit_defers_literal_on_new_string_format_placeholder() {
        // Existing anchored `string_format` with format "Hi {name}" — it has a live `name` pin but
        // no `age` pin (that placeholder does not exist yet).
        let mut board = empty_board();
        let mut event = Node::new("events_simple", "Start", "", "events");
        event.id = "event".to_string();
        event.set_start(true);
        event.add_output_pin("exec_out", "Out", "", VariableType::Execution);
        board.nodes.insert(event.id.clone(), event);

        let mut fmt = Node::new("string_format", "String Format", "", "std");
        fmt.id = "fmt".to_string();
        fmt.add_input_pin("format_string", "Format", "", VariableType::String)
            .default_value = Some("\"Hi {name}\"".to_string().into_bytes());
        fmt.add_input_pin("name", "Name", "", VariableType::Generic);
        fmt.add_output_pin("value", "Value", "", VariableType::String);
        board.nodes.insert(fmt.id.clone(), fmt);

        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "string_format",
                "String Format",
                vec![pin_meta("format_string", "String", PinType::Input)],
                vec![pin_meta("value", "String", PinType::Output)],
            ),
        ];

        // The re-submitted text adds a NEW `{age}` placeholder plus its literal.
        let result = reconcile_text_with_catalog(
            &board,
            "eventsSimple() {   //@n:event\n    stringFormat({ formatString: \"Hi {name} {age}\", name: \"World\", age: \"5\" })   //@n:fmt\n}\n",
            &catalog,
        );

        // The not-yet-minted `age` placeholder must NOT be reported as a missing pin...
        assert!(
            !result.diagnostics.iter().any(|d| d.contains("age")),
            "new placeholder should not diagnose as missing: {:?}",
            result.diagnostics
        );
        // ...its literal is deferred as an UpdateNodePin (apply mints the pin via on_update first).
        assert!(
            result.commands.iter().any(|command| matches!(
                command,
                BoardCommand::UpdateNodePin { node_id, pin_id, value, .. }
                    if node_id == "fmt"
                        && pin_id == "age"
                        && value == &flow_like_types::Value::String("5".to_string())
            )),
            "literal for the new `age` placeholder should be deferred as an UpdateNodePin: {:?}",
            result.commands
        );
    }

    fn accumulator_catalog() -> Vec<NodeMetadata> {
        vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "variable_get",
                "Get Variable",
                vec![pin_meta("var_ref", "String", PinType::Input)],
                vec![pin_meta("value_ref", "Generic", PinType::Output)],
            ),
            catalog_meta(
                "variable_set",
                "Set Variable",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("var_ref", "String", PinType::Input),
                    pin_meta("value_in", "Generic", PinType::Input),
                ],
                vec![
                    pin_meta("exec_out", "Execution", PinType::Output),
                    pin_meta("value_ref", "Generic", PinType::Output),
                ],
            ),
            catalog_meta(
                "array_push",
                "Push",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta_friendly("array_in", "Array", "Generic", "Array", PinType::Input),
                    pin_meta("value", "Generic", PinType::Input),
                ],
                vec![
                    pin_meta("exec_out", "Execution", PinType::Output),
                    pin_meta_friendly("array_out", "Array", "Generic", "Array", PinType::Output),
                ],
            ),
            catalog_meta(
                "control_for_each",
                "For Each",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta_friendly("array", "Array", "Generic", "Array", PinType::Input),
                ],
                vec![
                    pin_meta("exec_out", "Execution", PinType::Output),
                    pin_meta("value", "Generic", PinType::Output),
                    pin_meta("index", "Integer", PinType::Output),
                    pin_meta("done", "Execution", PinType::Output),
                ],
            ),
            catalog_meta(
                "batch_insert_local_db",
                "Batch Insert Local DB",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("database", "Struct", PinType::Input),
                    pin_meta_friendly("value", "Value", "Generic", "Array", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
        ]
    }

    fn struct_accumulator_catalog() -> Vec<NodeMetadata> {
        let mut catalog = accumulator_catalog();
        catalog.extend([
            catalog_meta(
                "struct_make",
                "Make Struct",
                Vec::new(),
                vec![pin_meta("struct", "Struct", PinType::Output)],
            ),
            catalog_meta(
                "struct_set",
                "Set Field",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("struct_in", "Struct", PinType::Input),
                    pin_meta("field", "String", PinType::Input),
                    pin_meta("value", "Generic", PinType::Input),
                ],
                vec![
                    pin_meta("exec_out", "Execution", PinType::Output),
                    pin_meta("struct_out", "Struct", PinType::Output),
                ],
            ),
            catalog_meta(
                "cuid",
                "CUID v2",
                Vec::new(),
                vec![pin_meta("cuid", "String", PinType::Output)],
            ),
        ]);
        catalog
    }

    fn command_node_type(commands: &[BoardCommand], ref_id: &str) -> Option<String> {
        commands.iter().find_map(|command| match command {
            BoardCommand::AddNode {
                node_type,
                ref_id: Some(id),
                ..
            } if id == ref_id => Some(node_type.clone()),
            _ => None,
        })
    }

    fn by_ref_catalog() -> Vec<NodeMetadata> {
        vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "variable_get",
                "Get Variable",
                vec![pin_meta("var_ref", "String", PinType::Input)],
                vec![pin_meta("value_ref", "Generic", PinType::Output)],
            ),
            catalog_meta(
                "array_clear_ref",
                "Clear (By Ref)",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("var_ref", "String", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "array_push_ref",
                "Push (By Ref)",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("var_ref", "String", PinType::Input),
                    pin_meta("value", "Generic", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "batch_upsert_local_db",
                "Batch Upsert",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("database", "Struct", PinType::Input),
                    pin_meta_friendly("id_row", "ID Column", "String", "Normal", PinType::Input),
                    pin_meta_friendly("value", "Value", "Generic", "Array", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
        ]
    }

    #[test]
    fn catalog_aware_reconcile_normalizes_by_ref_variable_names_to_ids() {
        let board = empty_board();
        let catalog = by_ref_catalog();

        let result = reconcile_text_with_catalog(
            &board,
            r#"const rows: Struct[] = []   //@v:var_rows

eventsSimple() {
    arrayClearRef({ varRef: "rows" })
    arrayPushRef({ varRef: "rows", value: {} })
    batchUpsertLocalDb({ database: {}, idRow: "id", value: rows })
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::UpdateNodePin { node_id, pin_id, value, .. }
                    if command_node_type(&result.commands, node_id).as_deref()
                        == Some("array_clear_ref")
                        && pin_id == "var_ref"
                        && value == &flow_like_types::Value::String("var_rows".to_string())
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::UpdateNodePin { node_id, pin_id, value, .. }
                    if command_node_type(&result.commands, node_id).as_deref()
                        == Some("array_push_ref")
                        && pin_id == "var_ref"
                        && value == &flow_like_types::Value::String("var_rows".to_string())
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::UpdateNodePin { node_id, pin_id, value, .. }
                    if command_node_type(&result.commands, node_id).as_deref()
                        == Some("batch_upsert_local_db")
                        && pin_id == "id_row"
                        && value == &flow_like_types::Value::String("id".to_string())
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if command_node_type(&result.commands, from_node).as_deref()
                        == Some("variable_get")
                        && from_pin == "value_ref"
                        && command_node_type(&result.commands, to_node).as_deref()
                            == Some("batch_upsert_local_db")
                        && to_pin == "value"
            )
        }));
    }

    #[test]
    fn catalog_aware_reconcile_wires_existing_anchored_variable_argument() {
        let mut board = empty_board();
        let mut variable = Variable::new("rows", VariableType::Struct, ValueType::Array);
        variable.id = "var_rows".to_string();
        board.variables.insert(variable.id.clone(), variable);

        let mut event = Node::new("events_simple", "Simple Event", "", "events");
        event.id = "event".to_string();
        event.set_start(true);
        event.add_output_pin("exec_out", "Out", "", VariableType::Execution);
        board.nodes.insert(event.id.clone(), event);

        let mut batch = Node::new("batch_upsert_local_db", "Batch Upsert", "", "data");
        batch.id = "batch".to_string();
        batch.add_input_pin("exec_in", "In", "", VariableType::Execution);
        batch.add_input_pin("database", "Database", "", VariableType::Struct);
        batch.add_input_pin("id_row", "ID Column", "", VariableType::String);
        let value_pin = batch.add_input_pin("value", "Value", "", VariableType::Generic);
        value_pin.value_type = ValueType::Array;
        batch.add_output_pin("exec_out", "Out", "", VariableType::Execution);
        board.nodes.insert(batch.id.clone(), batch);

        let result = reconcile_text_with_catalog(
            &board,
            r#"const rows: Struct[] = []   //@v:var_rows

eventsSimple() {   //@n:event
    batchUpsertLocalDb({ database: {}, idRow: "id", value: rows })   //@n:batch
}
"#,
            &by_ref_catalog(),
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::AddNode { node_type, .. } if node_type == "variable_get"
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::UpdateNodePin { node_id, pin_id, value, .. }
                    if command_node_type(&result.commands, node_id).as_deref()
                        == Some("variable_get")
                        && pin_id == "var_ref"
                        && value == &flow_like_types::Value::String("var_rows".to_string())
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if command_node_type(&result.commands, from_node).as_deref()
                        == Some("variable_get")
                        && from_pin == "value_ref"
                        && to_node == "batch"
                        && to_pin == "value"
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::UpdateNodePin { node_id, pin_id, value, .. }
                    if node_id == "batch"
                        && pin_id == "id_row"
                        && value == &flow_like_types::Value::String("id".to_string())
            )
        }));
    }

    #[test]
    fn catalog_aware_reconcile_recreates_missing_inline_request_chain_for_anchored_call() {
        let mut board = empty_board();

        let mut event = Node::new("events_simple", "Simple Event", "", "events");
        event.id = "event".to_string();
        event.set_start(true);
        event.add_output_pin("exec_out", "Out", "", VariableType::Execution);
        board.nodes.insert(event.id.clone(), event);

        let mut fetch = Node::new("http_fetch", "API Call", "", "http");
        fetch.id = "fetch".to_string();
        fetch.add_input_pin("exec_in", "In", "", VariableType::Execution);
        fetch.add_input_pin("request", "Request", "", VariableType::Struct);
        fetch.add_output_pin("exec_success", "Success", "", VariableType::Execution);
        fetch.add_output_pin("response", "Response", "", VariableType::Struct);
        board.nodes.insert(fetch.id.clone(), fetch);

        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "http_make_request",
                "Make Request",
                vec![
                    pin_meta("method", "String", PinType::Input),
                    pin_meta("url", "String", PinType::Input),
                ],
                vec![pin_meta("request", "Struct", PinType::Output)],
            ),
            catalog_meta(
                "http_set_header",
                "Set Header",
                vec![
                    pin_meta("request", "Struct", PinType::Input),
                    pin_meta("name", "String", PinType::Input),
                    pin_meta("value", "String", PinType::Input),
                ],
                vec![pin_meta("request", "Struct", PinType::Output)],
            ),
            catalog_meta(
                "http_fetch",
                "API Call",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("request", "Struct", PinType::Input),
                ],
                vec![
                    pin_meta("exec_success", "Execution", PinType::Output),
                    pin_meta("response", "Struct", PinType::Output),
                ],
            ),
        ];

        let result = reconcile_text_with_catalog(
            &board,
            r#"simpleEvent() {   //@n:event
    const aPICall = httpFetch({ request: httpSetHeader({ request: httpMakeRequest({ method: "GET", url: "https://www.reddit.com/r/rust/.rss" }), name: "User-Agent", value: "FlowLikeRedditRSS/1.0" }) })   //@n:fetch
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::AddNode { node_type, .. } if node_type == "http_make_request"
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::AddNode { node_type, .. } if node_type == "http_set_header"
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::UpdateNodePin { node_id, pin_id, value, .. }
                    if command_node_type(&result.commands, node_id).as_deref()
                        == Some("http_make_request")
                        && pin_id == "method"
                        && value == &flow_like_types::Value::String("GET".to_string())
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if command_node_type(&result.commands, from_node).as_deref()
                        == Some("http_make_request")
                        && from_pin == "request"
                        && command_node_type(&result.commands, to_node).as_deref()
                            == Some("http_set_header")
                        && to_pin == "request"
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if command_node_type(&result.commands, from_node).as_deref()
                        == Some("http_set_header")
                        && from_pin == "request"
                        && to_node == "fetch"
                        && to_pin == "request"
            )
        }));
    }

    #[test]
    fn catalog_aware_reconcile_does_not_remove_unanchored_inline_helpers() {
        let mut board = empty_board();

        let mut request = Node::new("http_make_request", "Make Request", "", "http");
        request.id = "request".to_string();
        request
            .add_input_pin("method", "Method", "", VariableType::String)
            .default_value = Some(flow_like_types::json::to_vec("GET").unwrap());
        request
            .add_input_pin("url", "URL", "", VariableType::String)
            .default_value =
            Some(flow_like_types::json::to_vec("https://www.reddit.com/r/rust/.rss").unwrap());
        let request_out = request
            .add_output_pin("request", "Request", "", VariableType::Struct)
            .id
            .clone();
        board.nodes.insert(request.id.clone(), request);

        let mut fetch = Node::new("http_fetch", "API Call", "", "http");
        fetch.id = "fetch".to_string();
        fetch.add_input_pin("exec_in", "In", "", VariableType::Execution);
        let fetch_request = fetch
            .add_input_pin("request", "Request", "", VariableType::Struct)
            .id
            .clone();
        fetch.add_output_pin("exec_success", "Success", "", VariableType::Execution);
        fetch.add_output_pin("response", "Response", "", VariableType::Struct);
        board.nodes.insert(fetch.id.clone(), fetch);

        connect(&mut board, "request", &request_out, "fetch", &fetch_request);

        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "http_make_request",
                "Make Request",
                vec![
                    pin_meta("method", "String", PinType::Input),
                    pin_meta("url", "String", PinType::Input),
                ],
                vec![pin_meta("request", "Struct", PinType::Output)],
            ),
            catalog_meta(
                "http_fetch",
                "API Call",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("request", "Struct", PinType::Input),
                ],
                vec![
                    pin_meta("exec_success", "Execution", PinType::Output),
                    pin_meta("response", "Struct", PinType::Output),
                ],
            ),
        ];

        let result = reconcile_text_with_catalog(
            &board,
            r#"simpleEvent() {
    const aPICall = httpFetch({ request: httpMakeRequest({ method: "GET", url: "https://www.reddit.com/r/rust/.rss" }) })   //@n:fetch
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(!result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::RemoveNode { node_id, .. } if node_id == "request"
            )
        }));
        assert!(!result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::AddNode { node_type, .. } if node_type == "http_make_request"
            )
        }));
    }

    #[test]
    fn catalog_aware_reconcile_repairs_anchored_variable_set_assignment() {
        let mut board = empty_board();
        let mut variable = Variable::new("rows", VariableType::Struct, ValueType::Array);
        variable.id = "var_rows".to_string();
        board.variables.insert(variable.id.clone(), variable);

        let mut push = Node::new("array_push", "Push", "", "array");
        push.id = "push".to_string();
        push.add_input_pin("exec_in", "In", "", VariableType::Execution);
        push.add_output_pin("exec_out", "Out", "", VariableType::Execution);
        push.add_output_pin("array_out", "Array", "", VariableType::Struct)
            .value_type = ValueType::Array;
        board.nodes.insert(push.id.clone(), push);

        let mut set_rows = Node::new("variable_set", "Set rows", "", "variables");
        set_rows.id = "set_rows".to_string();
        set_rows.add_input_pin("exec_in", "In", "", VariableType::Execution);
        set_rows.add_input_pin("var_ref", "rows", "", VariableType::String);
        let value_pin = set_rows.add_input_pin("value_in", "Value", "", VariableType::Struct);
        value_pin.value_type = ValueType::Array;
        set_rows.add_output_pin("exec_out", "Out", "", VariableType::Execution);
        set_rows
            .add_output_pin("value_ref", "New Value", "", VariableType::Struct)
            .value_type = ValueType::Array;
        board.nodes.insert(set_rows.id.clone(), set_rows);

        let result = reconcile_text_with_catalog(
            &board,
            r#"const rows: Struct[] = []   //@v:var_rows

push(arrayOut: Struct[]) {   //@n:push
    rows = arrayOut   //@n:set_rows
}
"#,
            &accumulator_catalog(),
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::UpdateNodePin { node_id, pin_id, value, .. }
                    if node_id == "set_rows"
                        && pin_id == "var_ref"
                        && value == &flow_like_types::Value::String("var_rows".to_string())
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if from_node == "push"
                        && from_pin == "array_out"
                        && to_node == "set_rows"
                        && to_pin == "value_in"
            )
        }));
    }

    #[test]
    fn catalog_aware_reconcile_wires_top_level_variable_accumulator_to_db_insert() {
        let board = empty_board();
        let catalog = accumulator_catalog();

        let result = reconcile_text_with_catalog(
            &board,
            r#"const rows: Struct[] = []

eventsSimple() {
    const push = arrayPush({ arrayIn: rows, value: { id: "one" } })
    rows = push.arrayOut
    batchInsertLocalDb({ database: {}, value: rows })
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::CreateVariable {
                    variable_id: Some(variable_id),
                    name,
                    value_type,
                    ..
                } if variable_id == "var_rows" && name == "rows" && value_type == "Array"
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::AddNode { node_type, .. } if node_type == "variable_get"
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::AddNode { node_type, .. } if node_type == "variable_set"
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::UpdateNodePin { pin_id, value, .. }
                    if pin_id == "var_ref"
                        && value == &flow_like_types::Value::String("var_rows".to_string())
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if command_node_type(&result.commands, from_node).as_deref() == Some("array_push")
                        && from_pin == "array_out"
                        && command_node_type(&result.commands, to_node).as_deref() == Some("variable_set")
                        && to_pin == "value_in"
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if command_node_type(&result.commands, from_node).as_deref() == Some("variable_get")
                        && from_pin == "value_ref"
                        && command_node_type(&result.commands, to_node).as_deref() == Some("batch_insert_local_db")
                        && to_pin == "value"
            )
        }));
    }

    #[test]
    fn catalog_aware_reconcile_wires_struct_set_chain_into_accumulator_insert() {
        let board = empty_board();
        let catalog = struct_accumulator_catalog();

        let result = reconcile_text_with_catalog(
            &board,
            r#"const rows: Struct[] = []

eventsSimple() {
    let row = structMake()
    row = structSet({ structIn: row, field: "id", value: cuid().cuid })
    row = structSet({ structIn: row, field: "subject", value: "hello" })
    const push = arrayPush({ arrayIn: rows, value: row })
    rows = push.arrayOut
    batchInsertLocalDb({ database: {}, value: rows })
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);

        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::UpdateNodePin { node_id, pin_id, value, .. }
                    if command_node_type(&result.commands, node_id).as_deref() == Some("struct_set")
                        && pin_id == "field"
                        && value == &flow_like_types::Value::String("id".to_string())
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::UpdateNodePin { node_id, pin_id, value, .. }
                    if command_node_type(&result.commands, node_id).as_deref() == Some("struct_set")
                        && pin_id == "field"
                        && value == &flow_like_types::Value::String("subject".to_string())
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::UpdateNodePin { node_id, pin_id, value, .. }
                    if command_node_type(&result.commands, node_id).as_deref() == Some("struct_set")
                        && pin_id == "value"
                        && value == &flow_like_types::Value::String("hello".to_string())
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if command_node_type(&result.commands, from_node).as_deref() == Some("cuid")
                        && from_pin == "cuid"
                        && command_node_type(&result.commands, to_node).as_deref() == Some("struct_set")
                        && to_pin == "value"
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if command_node_type(&result.commands, from_node).as_deref() == Some("struct_set")
                        && from_pin == "struct_out"
                        && command_node_type(&result.commands, to_node).as_deref() == Some("array_push")
                        && to_pin == "value"
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if command_node_type(&result.commands, from_node).as_deref() == Some("array_push")
                        && from_pin == "array_out"
                        && command_node_type(&result.commands, to_node).as_deref() == Some("variable_set")
                        && to_pin == "value_in"
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if command_node_type(&result.commands, from_node).as_deref() == Some("variable_get")
                        && from_pin == "value_ref"
                        && command_node_type(&result.commands, to_node).as_deref() == Some("batch_insert_local_db")
                        && to_pin == "value"
            )
        }));
    }

    #[test]
    fn member_assignment_sugar_reconciles_to_struct_set() {
        let board = empty_board();
        let catalog = struct_accumulator_catalog();

        let result = reconcile_text_with_catalog(
            &board,
            r#"eventsSimple() {
    let pref = structMake()
    pref.cost_weight = 0.5
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        // `pref.cost_weight = 0.5` desugars to a `struct_set` node with field="cost_weight".
        assert!(
            result.commands.iter().any(|command| {
                matches!(
                    command,
                    BoardCommand::UpdateNodePin { node_id, pin_id, value, .. }
                        if command_node_type(&result.commands, node_id).as_deref() == Some("struct_set")
                            && pin_id == "field"
                            && value == &flow_like_types::Value::String("cost_weight".to_string())
                )
            }),
            "expected a struct_set with field=\"cost_weight\"; commands: {:?}",
            result.commands
        );
        // The struct source feeds the struct_set's struct_in (the read-modify-write chain).
        assert!(
            result.commands.iter().any(|command| {
                matches!(
                    command,
                    BoardCommand::ConnectPins { from_node, to_node, to_pin, .. }
                        if command_node_type(&result.commands, from_node).as_deref() == Some("struct_make")
                            && command_node_type(&result.commands, to_node).as_deref() == Some("struct_set")
                            && to_pin == "struct_in"
                )
            }),
            "expected struct_make -> struct_set.struct_in; commands: {:?}",
            result.commands
        );
    }

    fn preferences_catalog() -> Vec<NodeMetadata> {
        vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "ai_generative_make_preferences",
                "Make Preferences",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("multimodal", "Boolean", PinType::Input),
                ],
                vec![
                    pin_meta("exec_out", "Execution", PinType::Output),
                    pin_meta("preferences", "Struct", PinType::Output),
                ],
            ),
            catalog_meta(
                "struct_set",
                "Set Field",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("struct_in", "Struct", PinType::Input),
                    pin_meta("field", "String", PinType::Input),
                    pin_meta("value", "Generic", PinType::Input),
                ],
                vec![
                    pin_meta("exec_out", "Execution", PinType::Output),
                    pin_meta("struct_out", "Struct", PinType::Output),
                ],
            ),
            catalog_meta(
                "ai_generative_find_model",
                "Find Model",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("preferences", "Struct", PinType::Input),
                ],
                vec![
                    pin_meta("exec_out", "Execution", PinType::Output),
                    pin_meta("model", "Struct", PinType::Output),
                ],
            ),
        ]
    }

    /// Every `ConnectPins` this reconcile emitted, as `(from_node, to_node)` — used to assert no
    /// edge is a self-connection, which `connect_pins` rejects at apply time with "Cannot connect a
    /// node to itself".
    fn self_connections(commands: &[BoardCommand]) -> Vec<(String, String)> {
        commands
            .iter()
            .filter_map(|command| match command {
                BoardCommand::ConnectPins {
                    from_node, to_node, ..
                } if from_node == to_node => Some((from_node.clone(), to_node.clone())),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn catalog_aware_reconcile_const_bound_struct_set_avoids_self_connection() {
        let board = empty_board();
        let catalog = preferences_catalog();

        // The struct-field write sugar has already been desugared by the parser into the
        // accumulator form `x = structSet({ structIn: x, … })`. `x` here is a `const`-bound impure
        // single-output node; the reassignment must wire struct_in from the PRIOR source and thread
        // the impure struct_set into the exec chain exactly once — never onto itself.
        let result = reconcile_text_with_catalog(
            &board,
            r#"eventsSimple() {
    const preferences = aiGenerativeMakePreferences({ multimodal: true })
    preferences = structSet({ structIn: preferences, field: "coding_weight", value: 0.5 })
    const findModel = aiGenerativeFindModel({ preferences: preferences })
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(
            self_connections(&result.commands).is_empty(),
            "reconcile emitted a self-connection: {:?}",
            self_connections(&result.commands)
        );

        // struct_in reads the pre-reassignment source (make_preferences.preferences), not struct_set.
        assert!(
            result.commands.iter().any(|command| matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if command_node_type(&result.commands, from_node).as_deref()
                        == Some("ai_generative_make_preferences")
                        && from_pin == "preferences"
                        && command_node_type(&result.commands, to_node).as_deref() == Some("struct_set")
                        && to_pin == "struct_in"
            )),
            "expected make_preferences.preferences -> struct_set.struct_in; commands: {:?}",
            result.commands
        );
        // The rebound `preferences` (struct_set.struct_out) feeds the downstream consumer.
        assert!(
            result.commands.iter().any(|command| matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if command_node_type(&result.commands, from_node).as_deref() == Some("struct_set")
                        && from_pin == "struct_out"
                        && command_node_type(&result.commands, to_node).as_deref()
                            == Some("ai_generative_find_model")
                        && to_pin == "preferences"
            )),
            "expected struct_set.struct_out -> find_model.preferences; commands: {:?}",
            result.commands
        );
        // The impure struct_set is threaded into the exec chain from make_preferences, once.
        let struct_set_exec_in: Vec<_> = result
            .commands
            .iter()
            .filter(|command| matches!(
                command,
                BoardCommand::ConnectPins { to_node, to_pin, .. }
                    if command_node_type(&result.commands, to_node).as_deref() == Some("struct_set")
                        && to_pin == "exec_in"
            ))
            .collect();
        assert_eq!(
            struct_set_exec_in.len(),
            1,
            "struct_set exec_in should be wired exactly once; got {struct_set_exec_in:?}"
        );
        assert!(
            matches!(
                struct_set_exec_in[0],
                BoardCommand::ConnectPins { from_node, .. }
                    if command_node_type(&result.commands, from_node).as_deref()
                        == Some("ai_generative_make_preferences")
            ),
            "struct_set exec_in should come from make_preferences; got {struct_set_exec_in:?}"
        );
    }

    #[test]
    fn member_assignment_sugar_on_const_binding_avoids_self_connection() {
        let board = empty_board();
        let catalog = preferences_catalog();

        // Same shape as above but exercising the parser sugar `x.field = value` directly, so the
        // desugaring path is covered end to end.
        let result = reconcile_text_with_catalog(
            &board,
            r#"eventsSimple() {
    const preferences = aiGenerativeMakePreferences({ multimodal: true })
    preferences.coding_weight = 0.5
    const findModel = aiGenerativeFindModel({ preferences: preferences })
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(
            self_connections(&result.commands).is_empty(),
            "reconcile emitted a self-connection: {:?}",
            self_connections(&result.commands)
        );
        assert!(
            result.commands.iter().any(|command| matches!(
                command,
                BoardCommand::UpdateNodePin { node_id, pin_id, value, .. }
                    if command_node_type(&result.commands, node_id).as_deref() == Some("struct_set")
                        && pin_id == "field"
                        && value == &flow_like_types::Value::String("coding_weight".to_string())
            )),
            "expected struct_set field=\"coding_weight\"; commands: {:?}",
            result.commands
        );
        assert!(
            result.commands.iter().any(|command| matches!(
                command,
                BoardCommand::ConnectPins { from_node, to_node, to_pin, .. }
                    if command_node_type(&result.commands, from_node).as_deref()
                        == Some("ai_generative_make_preferences")
                        && command_node_type(&result.commands, to_node).as_deref() == Some("struct_set")
                        && to_pin == "struct_in"
            )),
            "expected make_preferences -> struct_set.struct_in; commands: {:?}",
            result.commands
        );
    }

    /// Build an `eventsSimple → structSet(seed) → structSet(accumulate) → batchInsertLocalDb`
    /// board. The second `struct_set` reads the seed's `struct_out` (same accumulator) and rebinds
    /// it. With `wired_field == false` its `field` is a literal — the shape lowering sugars to
    /// `row.title = "hello"`; with `wired_field == true` a `cuid` node feeds `field`, so it must
    /// stay the explicit `structSet({…})` form.
    fn board_with_struct_accumulator(wired_field: bool) -> Board {
        let mut board = empty_board();

        let mut event = Node::new("events_simple", "", "", "events");
        event.id = "event".to_string();
        event.set_start(true);
        let ev_out = event
            .add_output_pin("exec_out", "Out", "", VariableType::Execution)
            .id
            .clone();
        board.nodes.insert(event.id.clone(), event);

        let mut seed = Node::new("struct_set", "Set Field", "", "structs");
        seed.id = "seed".to_string();
        let seed_exec_in = seed
            .add_input_pin("exec_in", "In", "", VariableType::Execution)
            .id
            .clone();
        let seed_exec_out = seed
            .add_output_pin("exec_out", "Out", "", VariableType::Execution)
            .id
            .clone();
        seed.add_input_pin("field", "Field", "", VariableType::String)
            .default_value = Some(br#""created""#.to_vec());
        seed.add_input_pin("value", "Value", "", VariableType::Generic)
            .default_value = Some(b"1".to_vec());
        let seed_struct_out = seed
            .add_output_pin("struct_out", "Struct Out", "", VariableType::Struct)
            .id
            .clone();
        board.nodes.insert(seed.id.clone(), seed);

        let mut acc = Node::new("struct_set", "Set Field", "", "structs");
        acc.id = "acc".to_string();
        let acc_exec_in = acc
            .add_input_pin("exec_in", "In", "", VariableType::Execution)
            .id
            .clone();
        let acc_exec_out = acc
            .add_output_pin("exec_out", "Out", "", VariableType::Execution)
            .id
            .clone();
        let acc_struct_in = acc
            .add_input_pin("struct_in", "Struct In", "", VariableType::Struct)
            .id
            .clone();
        let acc_field_id = {
            let field = acc.add_input_pin("field", "Field", "", VariableType::String);
            if !wired_field {
                field.default_value = Some(br#""title""#.to_vec());
            }
            field.id.clone()
        };
        acc.add_input_pin("value", "Value", "", VariableType::Generic)
            .default_value = Some(br#""hello""#.to_vec());
        let acc_struct_out = acc
            .add_output_pin("struct_out", "Struct Out", "", VariableType::Struct)
            .id
            .clone();
        board.nodes.insert(acc.id.clone(), acc);

        let mut sink = Node::new("batch_insert_local_db", "Batch Insert Local DB", "", "data");
        sink.id = "sink".to_string();
        let sink_exec_in = sink
            .add_input_pin("exec_in", "In", "", VariableType::Execution)
            .id
            .clone();
        let sink_value = sink
            .add_input_pin("value", "Value", "", VariableType::Struct)
            .id
            .clone();
        board.nodes.insert(sink.id.clone(), sink);

        connect(&mut board, "event", &ev_out, "seed", &seed_exec_in);
        connect(&mut board, "seed", &seed_exec_out, "acc", &acc_exec_in);
        connect(&mut board, "seed", &seed_struct_out, "acc", &acc_struct_in);
        connect(&mut board, "acc", &acc_exec_out, "sink", &sink_exec_in);
        connect(&mut board, "acc", &acc_struct_out, "sink", &sink_value);

        if wired_field {
            let mut cuid = Node::new("cuid", "CUID v2", "", "std");
            cuid.id = "cuid".to_string();
            let cuid_out = cuid
                .add_output_pin("cuid", "CUID", "", VariableType::String)
                .id
                .clone();
            board.nodes.insert(cuid.id.clone(), cuid);
            connect(&mut board, "cuid", &cuid_out, "acc", &acc_field_id);
        }

        board
    }

    #[test]
    fn lower_sugars_accumulator_reassignment_and_reconciles_back() {
        let board = board_with_struct_accumulator(false);
        let text =
            super::super::board_to_flowscript(&board, &flow_like_ast::RenderOptions::default());

        // The accumulator reassignment lowers to the dot-form struct-field write; only the seed
        // `struct_set` (which cannot be a `base.field =` write) keeps its explicit call.
        assert!(
            text.contains("row.title = \"hello\""),
            "accumulator reassignment must render as the dot form:\n{text}"
        );
        assert_eq!(
            text.matches("structSet(").count(),
            1,
            "only the seed struct_set stays explicit; the reassignment is sugared:\n{text}"
        );

        // Reconciling the rendered text against an empty board recreates the same struct_set shape:
        // a struct_set with a literal `field`, `struct_in` fed by the seed (base's prior source),
        // `struct_out` rebound into the consumer, and no self-connection.
        let catalog = struct_accumulator_catalog();
        let result = reconcile_text_with_catalog(&empty_board(), &text, &catalog);
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(
            self_connections(&result.commands).is_empty(),
            "reconcile emitted a self-connection: {:?}",
            self_connections(&result.commands)
        );
        assert!(
            result.commands.iter().any(|command| matches!(
                command,
                BoardCommand::UpdateNodePin { node_id, pin_id, value, .. }
                    if command_node_type(&result.commands, node_id).as_deref() == Some("struct_set")
                        && pin_id == "field"
                        && value == &flow_like_types::Value::String("title".to_string())
            )),
            "expected struct_set field=\"title\"; commands: {:?}",
            result.commands
        );
        // struct_in of the dot-form struct_set is fed by the seed struct_set (base's prior source).
        assert!(
            result.commands.iter().any(|command| matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if command_node_type(&result.commands, from_node).as_deref() == Some("struct_set")
                        && from_pin == "struct_out"
                        && command_node_type(&result.commands, to_node).as_deref() == Some("struct_set")
                        && to_pin == "struct_in"
            )),
            "expected struct_set.struct_out -> struct_set.struct_in; commands: {:?}",
            result.commands
        );
        // The rebound accumulator (struct_out) flows into the downstream consumer.
        assert!(
            result.commands.iter().any(|command| matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if command_node_type(&result.commands, from_node).as_deref() == Some("struct_set")
                        && from_pin == "struct_out"
                        && command_node_type(&result.commands, to_node).as_deref()
                            == Some("batch_insert_local_db")
                        && to_pin == "value"
            )),
            "expected struct_set.struct_out -> batch_insert_local_db.value; commands: {:?}",
            result.commands
        );
    }

    #[test]
    fn lower_keeps_wired_field_struct_set_explicit() {
        let board = board_with_struct_accumulator(true);
        let text =
            super::super::board_to_flowscript(&board, &flow_like_ast::RenderOptions::default());

        // A wired (non-literal) `field` cannot be a `row.<field> = value` write, so lowering
        // conservatively keeps BOTH struct_sets in the explicit `structSet({…})` form.
        assert!(
            text.contains("field: cuid()"),
            "a wired-field struct_set must keep its explicit call:\n{text}"
        );
        assert_eq!(
            text.matches("structSet(").count(),
            2,
            "neither the seed nor the wired-field struct_set may sugar to a dot write:\n{text}"
        );
        assert!(
            !text.contains("row.title"),
            "the wired-field update must not render as a dot write:\n{text}"
        );
    }

    #[test]
    fn catalog_aware_reconcile_promotes_loop_local_accumulator_for_db_insert() {
        let board = empty_board();
        let catalog = accumulator_catalog();

        let result = reconcile_text_with_catalog(
            &board,
            r#"eventsSimple() {
    let rows = []
    for (const item of controlForEach({ array: ["one"] })) {
        const push = arrayPush({ arrayIn: rows, value: item.value })
        rows = push.arrayOut
    }
    batchInsertLocalDb({ database: {}, value: rows })
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::CreateVariable {
                    variable_id: Some(variable_id),
                    name,
                    default_value: Some(flow_like_types::Value::Array(values)),
                    value_type,
                    ..
                } if variable_id == "var_rows"
                    && name == "rows"
                    && values.is_empty()
                    && value_type == "Array"
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if command_node_type(&result.commands, from_node).as_deref() == Some("control_for_each")
                        && from_pin == "done"
                        && command_node_type(&result.commands, to_node).as_deref() == Some("batch_insert_local_db")
                        && to_pin == "exec_in"
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if command_node_type(&result.commands, from_node).as_deref() == Some("array_push")
                        && from_pin == "array_out"
                        && command_node_type(&result.commands, to_node).as_deref() == Some("variable_set")
                        && to_pin == "value_in"
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if command_node_type(&result.commands, from_node).as_deref() == Some("variable_get")
                        && from_pin == "value_ref"
                        && command_node_type(&result.commands, to_node).as_deref() == Some("batch_insert_local_db")
                        && to_pin == "value"
            )
        }));
    }

    #[test]
    fn catalog_aware_reconcile_resolves_loop_bind_outputs() {
        let board = empty_board();
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "control_for_each",
                "For Each",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta_friendly("array", "Array", "Generic", "Array", PinType::Input),
                ],
                vec![
                    pin_meta("exec_out", "Execution", PinType::Output),
                    pin_meta("value", "Any", PinType::Output),
                    pin_meta("index", "Integer", PinType::Output),
                    pin_meta("done", "Execution", PinType::Output),
                ],
            ),
            catalog_meta(
                "log",
                "Log",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("text", "Any", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
        ];

        let result = reconcile_text_with_catalog(
            &board,
            r#"eventsSimple() {
    for (const item of controlForEach({ array: ["hello"] })) {
        log({ text: item.value })
    }
}
"#,
            &catalog,
        );

        assert!(
            result
                .diagnostics
                .iter()
                .all(|diagnostic| !diagnostic.contains("skipped connection")),
            "{:?}",
            result.diagnostics
        );
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if from_node == "$1"
                        && from_pin == "exec_out"
                        && to_node == "$2"
                        && to_pin == "exec_in"
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if from_node == "$1"
                        && from_pin == "value"
                        && to_node == "$2"
                        && to_pin == "text"
            )
        }));
    }

    #[test]
    fn catalog_aware_reconcile_wires_loop_array_body_and_done_outputs() {
        let board = empty_board();
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "mail_imap_inbox",
                "IMAP Inbox",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("connection", "Struct", PinType::Input),
                ],
                vec![
                    pin_meta("exec_out", "Execution", PinType::Output),
                    pin_meta("inbox_struct", "Struct", PinType::Output),
                ],
            ),
            catalog_meta(
                "mail_imap_list",
                "List Mails",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("inbox", "Struct", PinType::Input),
                ],
                vec![
                    pin_meta("exec_out", "Execution", PinType::Output),
                    pin_meta_friendly(
                        "emails",
                        "Email References",
                        "Struct",
                        "Array",
                        PinType::Output,
                    ),
                ],
            ),
            catalog_meta(
                "control_for_each",
                "For Each",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta_friendly("array", "Array", "Generic", "Array", PinType::Input),
                ],
                vec![
                    pin_meta("exec_out", "Execution", PinType::Output),
                    pin_meta("value", "Struct", PinType::Output),
                    pin_meta("index", "Integer", PinType::Output),
                    pin_meta("done", "Execution", PinType::Output),
                ],
            ),
            catalog_meta(
                "email_imap_inbox_fetch_mail",
                "Fetch Mail",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("email_ref", "Struct", PinType::Input),
                ],
                vec![
                    pin_meta("exec_out", "Execution", PinType::Output),
                    pin_meta("email", "Struct", PinType::Output),
                ],
            ),
            catalog_meta(
                "log",
                "Log",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("text", "Struct", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
        ];

        let result = reconcile_text_with_catalog(
            &board,
            r#"ingestGmail() {
    const inbox = mailImapInbox({ connection: {} })
    const refs = mailImapList({ inbox: inbox })
    for (const ref of controlForEach({ array: refs.emailReferences })) {
        const mail = emailImapInboxFetchMail({ emailRef: ref.value })
    }
    log({ text: inbox })
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::AddNode { node_type, ref_id, .. }
                    if node_type == "events_simple" && ref_id.as_deref() == Some("$0")
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if from_node == "$2"
                        && from_pin == "emails"
                        && to_node == "$3"
                        && to_pin == "array"
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if from_node == "$3"
                        && from_pin == "exec_out"
                        && to_node == "$4"
                        && to_pin == "exec_in"
            )
        }));
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if from_node == "$3"
                        && from_pin == "done"
                        && to_node == "$5"
                        && to_pin == "exec_in"
            )
        }));
    }

    #[test]
    fn catalog_aware_reconcile_prefers_success_exec_output_over_error() {
        let board = empty_board();
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "http_fetch",
                "API Call",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("request", "Struct", PinType::Input),
                ],
                vec![
                    pin_meta_friendly(
                        "exec_error",
                        "Error",
                        "Execution",
                        "Normal",
                        PinType::Output,
                    ),
                    pin_meta("response", "Struct", PinType::Output),
                    pin_meta_friendly(
                        "exec_success",
                        "Success",
                        "Execution",
                        "Normal",
                        PinType::Output,
                    ),
                ],
            ),
            catalog_meta(
                "http_response_to_text",
                "Response To Text",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("response", "Struct", PinType::Input),
                ],
                vec![
                    pin_meta("exec_out", "Execution", PinType::Output),
                    pin_meta("text", "String", PinType::Output),
                ],
            ),
        ];

        let result = reconcile_text_with_catalog(
            &board,
            r#"eventsSimple() {
    const fetched = httpFetch({ request: {} })
    httpResponseToText({ response: fetched.response })
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if from_node == "$1"
                        && from_pin == "exec_success"
                        && to_node == "$2"
                        && to_pin == "exec_in"
            )
        }));
        assert!(!result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if from_node == "$1"
                        && from_pin == "exec_error"
                        && to_node == "$2"
                        && to_pin == "exec_in"
            )
        }));
    }

    #[test]
    fn catalog_aware_reconcile_keeps_exec_cursor_across_pure_nodes_and_loop_body() {
        let board = empty_board();
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "http_make_request",
                "Make Request",
                vec![
                    pin_meta("method", "String", PinType::Input),
                    pin_meta("url", "String", PinType::Input),
                ],
                vec![pin_meta("request", "Struct", PinType::Output)],
            ),
            catalog_meta(
                "http_fetch",
                "API Call",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("request", "Struct", PinType::Input),
                ],
                vec![
                    pin_meta("exec_success", "Execution", PinType::Output),
                    pin_meta("exec_error", "Execution", PinType::Output),
                    pin_meta("response", "Struct", PinType::Output),
                ],
            ),
            catalog_meta(
                "http_response_to_text",
                "To Text",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("response", "Struct", PinType::Input),
                ],
                vec![
                    pin_meta("exec_out", "Execution", PinType::Output),
                    pin_meta("text", "String", PinType::Output),
                ],
            ),
            catalog_meta(
                "parse_feed",
                "Parse Feed",
                vec![
                    pin_meta("feed_body", "String", PinType::Input),
                    pin_meta("source_url", "String", PinType::Input),
                ],
                vec![
                    pin_meta("items", "Struct", PinType::Output),
                    pin_meta("item_count", "Integer", PinType::Output),
                ],
            ),
            catalog_meta(
                "utils_datetime_now",
                "Now",
                vec![pin_meta("exec_in", "Execution", PinType::Input)],
                vec![
                    pin_meta("exec_out", "Execution", PinType::Output),
                    pin_meta("date", "Date", PinType::Output),
                ],
            ),
            catalog_meta(
                "utils_datetime_duration",
                "Add Duration",
                vec![
                    pin_meta("date", "Date", PinType::Input),
                    pin_meta("days", "Integer", PinType::Input),
                ],
                vec![pin_meta("result", "Date", PinType::Output)],
            ),
            catalog_meta(
                "filter_feed_items_by_date",
                "Filter Feed Items by Date",
                vec![
                    pin_meta_friendly("items", "Items", "Struct", "Array", PinType::Input),
                    pin_meta("released_from", "Date", PinType::Input),
                    pin_meta("released_to", "Date", PinType::Input),
                    pin_meta("date_field", "String", PinType::Input),
                    pin_meta("include_undated", "Boolean", PinType::Input),
                ],
                vec![
                    pin_meta_friendly(
                        "filtered_items",
                        "Filtered Items",
                        "Struct",
                        "Array",
                        PinType::Output,
                    ),
                    pin_meta("item_count", "Integer", PinType::Output),
                ],
            ),
            catalog_meta(
                "log_info",
                "Print Info",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("message", "Generic", PinType::Input),
                    pin_meta("toast", "Boolean", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "control_for_each",
                "For Each",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta_friendly("array", "Array", "Generic", "Array", PinType::Input),
                ],
                vec![
                    pin_meta("exec_out", "Execution", PinType::Output),
                    pin_meta("done", "Execution", PinType::Output),
                    pin_meta("value", "Generic", PinType::Output),
                    pin_meta("index", "Integer", PinType::Output),
                ],
            ),
            catalog_meta(
                "feed_item_to_markdown",
                "Feed Item to Markdown",
                vec![
                    pin_meta("item", "Generic", PinType::Input),
                    pin_meta("include_link", "Boolean", PinType::Input),
                    pin_meta("include_summary", "Boolean", PinType::Input),
                    pin_meta("include_content", "Boolean", PinType::Input),
                ],
                vec![pin_meta("markdown", "String", PinType::Output)],
            ),
        ];

        let result = reconcile_text_with_catalog(
            &board,
            r#"eventsSimple() {
    const request = httpMakeRequest({ method: "GET", url: "https://blog.rust-lang.org/feed.xml?limit=30" })
    const response = httpFetch({ request: request })
    const feedBody = httpResponseToText({ response: response.response })
    const parsed = parseFeed({ feedBody: feedBody, sourceUrl: "https://blog.rust-lang.org/feed.xml?limit=30" })
    const now = utilsDatetimeNow()
    const from = utilsDatetimeDuration({ date: now, days: -2 })
    const recent = filterFeedItemsByDate({ items: parsed.items, releasedFrom: from, releasedTo: now, dateField: "published", includeUndated: false })
    logInfo({ message: recent.itemCount, toast: false })
    for (const item of controlForEach({ array: recent.filteredItems })) {
        const markdown = feedItemToMarkdown({ item: item.value, includeLink: true, includeSummary: true, includeContent: false })
        logInfo({ message: markdown, toast: false })
    }
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        for (from_node, from_pin, to_node, to_pin) in [
            ("$0", "exec_out", "$2", "exec_in"),
            ("$2", "exec_success", "$3", "exec_in"),
            ("$3", "exec_out", "$5", "exec_in"),
            ("$5", "exec_out", "$8", "exec_in"),
            ("$8", "exec_out", "$9", "exec_in"),
            ("$9", "exec_out", "$11", "exec_in"),
        ] {
            assert!(
                result.commands.iter().any(|command| {
                    matches!(
                        command,
                        BoardCommand::ConnectPins {
                            from_node: actual_from_node,
                            from_pin: actual_from_pin,
                            to_node: actual_to_node,
                            to_pin: actual_to_pin,
                            ..
                        } if actual_from_node == from_node
                            && actual_from_pin == from_pin
                            && actual_to_node == to_node
                            && actual_to_pin == to_pin
                    )
                }),
                "missing exec edge {from_node}.{from_pin} -> {to_node}.{to_pin}; commands: {:?}",
                result.commands
            );
        }
    }

    #[test]
    fn catalog_aware_reconcile_does_not_guess_unknown_multi_exec_output() {
        let board = empty_board();
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "custom_split",
                "Custom Split",
                vec![pin_meta("exec_in", "Execution", PinType::Input)],
                vec![
                    pin_meta("exec_error", "Execution", PinType::Output),
                    pin_meta("exec_success", "Execution", PinType::Output),
                ],
            ),
            catalog_meta(
                "log_info",
                "Log Info",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("message", "String", PinType::Input),
                ],
                Vec::new(),
            ),
        ];

        let result = reconcile_text_with_catalog(
            &board,
            r#"eventsSimple() {
    customSplit()
    logInfo({ message: "done" })
}
"#,
            &catalog,
        );

        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.contains("multiple execution outputs")),
            "{:?}",
            result.diagnostics
        );
        assert!(!result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::ConnectPins { from_node, to_node, .. }
                    if from_node == "$1" && to_node == "$2"
            )
        }));
    }

    #[test]
    fn catalog_aware_reconcile_continues_unknown_streaming_nodes_from_exec_done() {
        let board = empty_board();
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "custom_stream",
                "Custom Stream",
                vec![pin_meta("exec_in", "Execution", PinType::Input)],
                vec![
                    pin_meta("on_stream", "Execution", PinType::Output),
                    pin_meta("exec_done", "Execution", PinType::Output),
                    pin_meta("result", "String", PinType::Output),
                ],
            ),
            catalog_meta(
                "log_info",
                "Log Info",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("message", "String", PinType::Input),
                ],
                Vec::new(),
            ),
        ];

        let result = reconcile_text_with_catalog(
            &board,
            r#"eventsSimple() {
    const streamed = customStream()
    logInfo({ message: streamed.result })
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, to_pin, .. }
                    if from_node == "$1"
                        && from_pin == "exec_done"
                        && to_node == "$2"
                        && to_pin == "exec_in"
            )
        }));
        assert!(!result.commands.iter().any(|command| {
            matches!(
                command,
                BoardCommand::ConnectPins { from_node, from_pin, to_node, .. }
                    if from_node == "$1" && from_pin == "on_stream" && to_node == "$2"
            )
        }));
    }

    #[test]
    fn catalog_aware_reconcile_wires_stream_chunk_consumers_from_on_stream_only() {
        let board = empty_board();
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "agent_stream_invoke",
                "Stream Invoke Agent",
                vec![pin_meta("exec_in", "Execution", PinType::Input)],
                vec![
                    pin_meta("on_stream", "Execution", PinType::Output),
                    pin_meta("chunk", "Struct", PinType::Output),
                    pin_meta("exec_done", "Execution", PinType::Output),
                    pin_meta("response", "Struct", PinType::Output),
                    pin_meta("stats", "Struct", PinType::Output),
                ],
            ),
            catalog_meta(
                "events_chat_push_response_chunk",
                "Push Chunk",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("chunk", "Struct", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "events_chat_push_response",
                "Push Response",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("response", "Struct", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "events_chat_push_stat",
                "Push Stat",
                vec![
                    pin_meta("exec_in", "Execution", PinType::Input),
                    pin_meta("stat", "Struct", PinType::Input),
                ],
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
        ];

        let result = reconcile_text_with_catalog(
            &board,
            r#"eventsSimple() {
    const invokeAgent = agentStreamInvoke()
    eventsChatPushResponseChunk({ chunk: invokeAgent.chunk })
    eventsChatPushResponse({ response: invokeAgent.response })
    eventsChatPushStat({ stat: invokeAgent.stats })
}
"#,
            &catalog,
        );

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        for (from_node, from_pin, to_node, to_pin) in [
            ("$0", "exec_out", "$1", "exec_in"),
            ("$1", "on_stream", "$2", "exec_in"),
            ("$1", "exec_done", "$3", "exec_in"),
            ("$3", "exec_out", "$4", "exec_in"),
        ] {
            assert!(
                result.commands.iter().any(|command| {
                    matches!(
                        command,
                        BoardCommand::ConnectPins {
                            from_node: actual_from_node,
                            from_pin: actual_from_pin,
                            to_node: actual_to_node,
                            to_pin: actual_to_pin,
                            ..
                        } if actual_from_node == from_node
                            && actual_from_pin == from_pin
                            && actual_to_node == to_node
                            && actual_to_pin == to_pin
                    )
                }),
                "missing exec edge {from_node}.{from_pin} -> {to_node}.{to_pin}; commands: {:?}",
                result.commands
            );
        }

        for (from_node, from_pin, to_node) in [
            ("$1", "exec_done", "$2"),
            ("$2", "exec_out", "$3"),
            ("$1", "on_stream", "$3"),
        ] {
            assert!(!result.commands.iter().any(|command| {
                matches!(
                    command,
                    BoardCommand::ConnectPins { from_node: actual_from_node, from_pin: actual_from_pin, to_node: actual_to_node, .. }
                        if actual_from_node == from_node
                            && actual_from_pin == from_pin
                            && actual_to_node == to_node
                )
            }));
        }
    }

    #[test]
    fn catalog_aware_reconcile_reports_missing_variable_refs() {
        let board = empty_board();
        let catalog = vec![
            catalog_meta(
                "events_simple",
                "Simple Event",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution", PinType::Output)],
            ),
            catalog_meta(
                "variable_get",
                "Get Variable",
                vec![pin_meta("var_ref", "String", PinType::Input)],
                vec![pin_meta("value_ref", "String", PinType::Output)],
            ),
        ];

        let result = reconcile_text_with_catalog(
            &board,
            r#"eventsSimple() {
    variableGet({ varRef: "GMAIL_ADDRESS" })
}
"#,
            &catalog,
        );

        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.contains("GMAIL_ADDRESS")
                    && diagnostic.contains("top-level FlowScript variable")),
            "{:?}",
            result.diagnostics
        );
    }

    /// Build `event → makePreferences → structSet(a) → structSet(b)` where the tail `structSet`
    /// (`set_b`) is a single-field accumulator update of the head's `struct_out`, so lowering
    /// re-sugars it to the `record.reasoning_weight = 0.5` dot form (a `Stmt::FieldAssign`).
    fn board_with_struct_accumulator_chain() -> Board {
        let mut board = empty_board();

        let mut event = Node::new("events_simple", "Start", "", "events");
        event.id = "event".to_string();
        event.set_start(true);
        let event_out = event
            .add_output_pin("exec_out", "Out", "", VariableType::Execution)
            .id
            .clone();
        board.nodes.insert(event.id.clone(), event);

        let mut make = Node::new("make_preferences", "Make Preferences", "", "data");
        make.id = "make".to_string();
        let make_in = make
            .add_input_pin("exec_in", "In", "", VariableType::Execution)
            .id
            .clone();
        let make_exec_out = make
            .add_output_pin("exec_out", "Out", "", VariableType::Execution)
            .id
            .clone();
        let make_struct_out = make
            .add_output_pin("preferences", "Preferences", "", VariableType::Struct)
            .id
            .clone();
        board.nodes.insert(make.id.clone(), make);

        let mut set_a = Node::new("struct_set", "Set Field", "", "structs");
        set_a.id = "set_a".to_string();
        let a_in = set_a
            .add_input_pin("exec_in", "In", "", VariableType::Execution)
            .id
            .clone();
        let a_struct_in = set_a
            .add_input_pin("struct_in", "Struct", "", VariableType::Struct)
            .id
            .clone();
        set_a
            .add_input_pin("field", "Field", "", VariableType::String)
            .default_value = Some(b"\"coding_weight\"".to_vec());
        set_a
            .add_input_pin("value", "Value", "", VariableType::Generic)
            .default_value = Some(b"0.3".to_vec());
        let a_exec_out = set_a
            .add_output_pin("exec_out", "Out", "", VariableType::Execution)
            .id
            .clone();
        let a_struct_out = set_a
            .add_output_pin("struct_out", "Struct", "", VariableType::Struct)
            .id
            .clone();
        board.nodes.insert(set_a.id.clone(), set_a);

        let mut set_b = Node::new("struct_set", "Set Field", "", "structs");
        set_b.id = "set_b".to_string();
        let b_in = set_b
            .add_input_pin("exec_in", "In", "", VariableType::Execution)
            .id
            .clone();
        let b_struct_in = set_b
            .add_input_pin("struct_in", "Struct", "", VariableType::Struct)
            .id
            .clone();
        set_b
            .add_input_pin("field", "Field", "", VariableType::String)
            .default_value = Some(b"\"reasoning_weight\"".to_vec());
        set_b
            .add_input_pin("value", "Value", "", VariableType::Generic)
            .default_value = Some(b"0.5".to_vec());
        set_b.add_output_pin("exec_out", "Out", "", VariableType::Execution);
        set_b.add_output_pin("struct_out", "Struct", "", VariableType::Struct);
        board.nodes.insert(set_b.id.clone(), set_b);

        connect(&mut board, "event", &event_out, "make", &make_in);
        connect(&mut board, "make", &make_exec_out, "set_a", &a_in);
        connect(&mut board, "make", &make_struct_out, "set_a", &a_struct_in);
        connect(&mut board, "set_a", &a_exec_out, "set_b", &b_in);
        connect(&mut board, "set_a", &a_struct_out, "set_b", &b_struct_in);
        board
    }

    #[test]
    fn field_assign_value_literal_edit_updates_struct_set_value_pin() {
        // Editing the value literal on an existing anchored `record.reasoning_weight = 0.5` dot
        // form (rendered from the tail struct_set accumulator) must update that struct_set's
        // `value` pin in place — the regression this synthesis fixes was a silent no-op here.
        let board = board_with_struct_accumulator_chain();
        let text =
            anchored_text(&board).replace("reasoning_weight = 0.5", "reasoning_weight = 0.7");
        let result = reconcile_text(&board, &text);

        assert!(
            result.commands.iter().any(|command| matches!(
                command,
                BoardCommand::UpdateNodePin { node_id, pin_id, value, .. }
                    if node_id == "set_b" && pin_id == "value" && value.as_f64() == Some(0.7)
            )),
            "expected UpdateNodePin on set_b.value = 0.7; got {:?}",
            result.commands
        );
        // The struct_set already exists; the edit must not duplicate or remove the accumulator node.
        assert!(
            !result.commands.iter().any(|command| matches!(
                command,
                BoardCommand::AddNode { .. } | BoardCommand::RemoveNode { .. }
            )),
            "field-assign config edit must not add or remove nodes; got {:?}",
            result.commands
        );
    }

    #[test]
    fn deleting_field_assign_line_removes_struct_set_node() {
        // Removing the anchored dot-form line must flag its struct_set for deletion — before the
        // synthesis, `visible` never saw the FieldAssign's node, so it lingered on the board.
        let board = board_with_struct_accumulator_chain();
        let reduced = anchored_text(&board)
            .lines()
            .filter(|line| !line.contains("//@n:set_b"))
            .collect::<Vec<_>>()
            .join("\n");
        let result = reconcile_text(&board, &reduced);

        let removed: Vec<_> = result
            .commands
            .iter()
            .filter_map(|command| match command {
                BoardCommand::RemoveNode { node_id, .. } => Some(node_id.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            removed,
            vec!["set_b"],
            "deleting the `record.reasoning_weight = …` line must remove only its struct_set; got {:?}",
            result.commands
        );
    }

    #[test]
    fn unchanged_field_assign_roundtrip_emits_nothing() {
        // Reconciling the identical anchored dot-form text against its own board must stay a no-op;
        // this guards against the synthesized struct_set call producing a spurious edit or deletion.
        let board = board_with_struct_accumulator_chain();
        let text = anchored_text(&board);
        let result = reconcile_text(&board, &text);
        assert!(
            result.commands.is_empty(),
            "no-op field-assign round-trip must emit no commands; got {:?} from text:\n{text}",
            result.commands
        );
    }
}
