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
use crate::flow::copilot::{BoardCommand, NodeMetadata, NodePosition, PinMetadata};
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
    reconcile_inner(existing, new, None)
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
    reconcile_inner(existing, new, Some(catalog))
}

fn reconcile_inner(
    existing: &Board,
    new: &BoardAst,
    catalog: Option<&[NodeMetadata]>,
) -> ReconcileResult {
    let mut result = ReconcileResult::default();
    let board_ast = super::lower_to_ast(existing);

    let variable_changes = reconcile_variables(&board_ast, new);
    result.commands.extend(variable_changes.commands);
    result.diagnostics.extend(variable_changes.diagnostics);

    // 1. Index every anchored call in the new AST by node id.
    let mut new_calls: HashMap<String, &Call> = HashMap::new();
    collect_calls(new, &mut new_calls);

    // 2. Configuration edits: for each anchored call that still maps to a board node, diff its
    //    literal arguments against the node's current pin defaults.
    for (anchor, call) in &new_calls {
        let Some(node) = find_board_node(existing, anchor) else {
            result.diagnostics.push(format!(
                "anchor {anchor} no longer resolves to a board node; skipped"
            ));
            continue;
        };
        for arg in &call.args {
            let Expr::Literal(lit) = &arg.value else {
                // References / nested calls describe wiring, which v1 does not rewrite.
                continue;
            };
            let Some(pin) = find_input_pin(node, &arg.name) else {
                result.diagnostics.push(format!(
                    "node {anchor} has no input pin named {:?}; skipped",
                    arg.name
                ));
                continue;
            };
            let new_value = literal_to_value(lit);
            let current = pin
                .default_value
                .as_deref()
                .and_then(|b| flow_like_types::json::from_slice::<flow_like_types::Value>(b).ok());
            if current.as_ref() == Some(&new_value) {
                continue; // unchanged
            }
            result.commands.push(BoardCommand::UpdateNodePin {
                node_id: anchor.clone(),
                pin_id: pin.name.clone(),
                value: new_value,
                summary: Some(format!("Set {} on {}", arg.name, node.friendly_name)),
            });
        }
    }

    // 3. Deletions: a text-visible anchored node absent from the new AST is a removal. We compute
    //    "text-visible" from the board's own lowered AST so sugared/inlined nodes (reroutes,
    //    struct_make, pure helpers) are never removed just for lacking an anchor in the text.
    let mut visible: HashMap<String, &Call> = HashMap::new();
    collect_calls(&board_ast, &mut visible);
    let new_anchors: HashSet<&String> = new_calls.keys().collect();
    for anchor in visible.keys() {
        if new_anchors.contains(anchor) {
            continue;
        }
        let Some(node) = find_board_node(existing, anchor) else {
            continue;
        };
        result.commands.push(BoardCommand::RemoveNode {
            node_id: anchor.clone(),
            summary: Some(format!("Remove {}", node.friendly_name)),
        });
    }

    // 4. Structural authoring: catalog-aware FlowPilot calls can add new unanchored calls in the
    //    text. Translate those to the same command format the UI already reviews/applies.
    if let Some(catalog) = catalog {
        let structural = StructuralPlanner::new(existing, catalog).plan(new);
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

    result
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
            None if var.anchor.is_some() => {
                result.diagnostics.push(format!(
                    "variable anchor {} no longer resolves to a board variable; skipped",
                    var.anchor.as_deref().unwrap_or_default()
                ));
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

fn find_input_pin<'a>(node: &'a Node, name: &str) -> Option<&'a Pin> {
    node.pins
        .values()
        .find(|p| p.pin_type == PinType::Input && node_pin_name_matches(p, name))
}

fn find_output_pin<'a>(node: &'a Node, name: &str) -> Option<&'a Pin> {
    node.pins
        .values()
        .find(|p| p.pin_type == PinType::Output && node_pin_name_matches(p, name))
}

fn pin_name_matches(raw: &str, requested: &str) -> bool {
    raw == requested || to_camel_case(raw) == requested
}

fn node_pin_name_matches(pin: &Pin, requested: &str) -> bool {
    pin_name_matches(&pin.name, requested) || pin_name_matches(&pin.friendly_name, requested)
}

fn metadata_pin_name_matches(pin: &PinMetadata, requested: &str) -> bool {
    pin_name_matches(&pin.name, requested) || pin_name_matches(&pin.friendly_name, requested)
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
    meta.inputs
        .iter()
        .find(|p| p.data_type != "Execution" && metadata_pin_name_matches(p, name))
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
            None
        }
    }
}

fn select_exec_success(candidates: &[ExecPinCandidate]) -> Option<String> {
    select_named_exec_pin(candidates, &["exec_success"])
}

fn select_named_exec_pin(candidates: &[ExecPinCandidate], names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| {
        candidates
            .iter()
            .find(|pin| pin.name == *name || pin.friendly_name == *name)
            .map(|pin| pin.name.clone())
    })
}

#[derive(Debug, Clone)]
enum NodeEntity {
    Existing(String),
    New { ref_id: String, meta: NodeMetadata },
}

impl NodeEntity {
    fn node_ref(&self) -> String {
        match self {
            Self::Existing(id) => id.clone(),
            Self::New { ref_id, .. } => ref_id.clone(),
        }
    }
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
}

impl PlannedStmt {
    fn new(entity: NodeEntity) -> Self {
        Self {
            entity,
            next_exec_pin: None,
        }
    }

    fn with_next_exec_pin(entity: NodeEntity, next_exec_pin: Option<String>) -> Self {
        Self {
            entity,
            next_exec_pin,
        }
    }

    fn next_cursor(self) -> ExecCursor {
        ExecCursor::with_output(self.entity, self.next_exec_pin)
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
    next_ref: usize,
    next_position: usize,
}

impl<'a> StructuralPlanner<'a> {
    fn new(existing: &'a Board, catalog: &[NodeMetadata]) -> Self {
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
            next_ref: 0,
            next_position: 0,
        }
    }

    fn plan(mut self, ast: &BoardAst) -> ReconcileResult {
        self.push_scope();
        self.seed_top_level_variables(ast);
        for event in &ast.events {
            self.plan_event(event);
        }
        for func in &ast.functions {
            self.plan_function(func);
        }
        self.pop_scope();

        self.result.commands.extend(self.add_commands);
        // Literal/config updates can change node shape (for example format/schema/template
        // nodes whose on_update adds dynamic pins), so apply them before resolving connections.
        self.result.commands.extend(self.update_commands);
        self.result.commands.extend(self.disconnect_commands);
        self.result.commands.extend(self.connect_commands);
        self.result
    }

    fn plan_event(&mut self, event: &EventBlock) {
        let entry = match &event.anchor {
            Some(anchor) => {
                find_board_node(self.existing, anchor).map(|_| NodeEntity::Existing(anchor.clone()))
            }
            None => self.add_entry_node(&event.name),
        };

        self.push_scope();
        if let Some(entry) = &entry {
            self.seed_params_from_entity(&event.params, entry);
        }
        self.plan_block(&event.body, entry.map(ExecCursor::new), None);
        self.pop_scope();
    }

    fn plan_function(&mut self, func: &FnDecl) {
        let target_layer = func.anchor.clone();
        self.push_scope();
        self.plan_block(&func.body, None, target_layer);
        self.pop_scope();
    }

    fn plan_block(
        &mut self,
        block: &Block,
        entry: Option<ExecCursor>,
        target_layer: Option<String>,
    ) {
        let mut previous_exec = entry;
        let mut inserted_since_existing = false;
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
            let Some(current) = planned else {
                continue;
            };

            let accepts_exec = self.entity_exec_input_pin(&current.entity).is_some();
            let continues_exec = current.next_exec_pin.is_some()
                || (accepts_exec && !self.entity_exec_output_pins(&current.entity).is_empty());

            if accepts_exec {
                if let Some(previous) = &previous_exec {
                    if self.connect_exec(previous, &current.entity, inserted_since_existing) {
                        inserted_since_existing |= matches!(current.entity, NodeEntity::New { .. });
                    }
                } else {
                    inserted_since_existing |= matches!(current.entity, NodeEntity::New { .. });
                }
            }

            if matches!(current.entity, NodeEntity::Existing(_)) {
                inserted_since_existing = false;
            }
            if continues_exec {
                previous_exec = Some(current.next_cursor());
            } else if accepts_exec {
                previous_exec = None;
            }
        }
    }

    fn plan_stmt(
        &mut self,
        stmt: &Stmt,
        target_layer: Option<String>,
        promote_local_alias: bool,
    ) -> Option<PlannedStmt> {
        match stmt {
            Stmt::Let { name, call, anchor } => {
                let entity = self.plan_call_statement(call, anchor.as_deref(), target_layer);
                if let Some(entity) = &entity {
                    self.insert_symbol(
                        name.clone(),
                        SymbolValue::Source(ValueSource {
                            node: entity.clone(),
                            output_pin: None,
                        }),
                    );
                }
                entity.map(PlannedStmt::new)
            }
            Stmt::Call { call, anchor } => self
                .plan_call_statement(call, anchor.as_deref(), target_layer)
                .map(PlannedStmt::new),
            Stmt::Assign {
                target,
                value,
                anchor,
            } => {
                if let Some(anchor) = anchor {
                    let entity = NodeEntity::Existing(anchor.clone());
                    if let Some(output_pin) = assigned_call_output_pin(value)
                        && find_board_node(self.existing, anchor).is_some()
                    {
                        self.insert_symbol(
                            target.clone(),
                            SymbolValue::Source(ValueSource {
                                node: entity.clone(),
                                output_pin: Some(output_pin.to_string()),
                            }),
                        );
                    }
                    return Some(PlannedStmt::new(entity));
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
                self.assign_symbol(target.clone(), resolved);
                entity.map(PlannedStmt::new)
            }
            Stmt::LocalAlias {
                name,
                value,
                anchor,
            } => {
                if let Some(anchor) = anchor {
                    let entity = NodeEntity::Existing(anchor.clone());
                    if let Some(output_pin) = assigned_call_output_pin(value)
                        && find_board_node(self.existing, anchor).is_some()
                    {
                        self.insert_symbol(
                            name.clone(),
                            SymbolValue::Source(ValueSource {
                                node: entity.clone(),
                                output_pin: Some(output_pin.to_string()),
                            }),
                        );
                    }
                    return Some(PlannedStmt::new(entity));
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

                if anchor.is_none() && bind.is_none() {
                    self.result.diagnostics.push(
                        "new FlowScript branch statements are not yet converted automatically; use the control_branch declaration or emit_commands for complex branch wiring".to_string(),
                    );
                }
                if let Some(cond) = condition {
                    let _ = self.resolve_expr(cond, target_layer.clone());
                }
                for arm in arms {
                    self.push_scope();
                    self.plan_block(
                        &arm.body,
                        entity.clone().map(ExecCursor::new),
                        target_layer.clone(),
                    );
                    self.pop_scope();
                }
                if bind.is_some() && is_placeholder_call(call) {
                    None
                } else {
                    entity.map(PlannedStmt::new)
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
                entity.map(|entity| {
                    PlannedStmt::with_next_exec_pin(
                        entity.clone(),
                        self.entity_exec_output_pin_named(&entity, &["done", "exec_done"]),
                    )
                })
            }
            Stmt::Handler(event) => {
                self.plan_event(event);
                None
            }
            Stmt::Local(_) | Stmt::Return { .. } | Stmt::Comment(_) => None,
        }
    }

    fn plan_call_statement(
        &mut self,
        call: &Call,
        anchor: Option<&str>,
        target_layer: Option<String>,
    ) -> Option<NodeEntity> {
        if let Some(anchor) = anchor {
            return find_board_node(self.existing, anchor)
                .map(|_| NodeEntity::Existing(anchor.to_string()));
        }

        if call.display.trim().is_empty() {
            return None;
        }

        self.add_call_node(call, target_layer)
    }

    fn add_entry_node(&mut self, display: &str) -> Option<NodeEntity> {
        match self.catalog.resolve_display(display) {
            Ok(meta) => Some(self.queue_add_node(meta, None)),
            Err(err) => {
                for fallback in ["eventsSimple", "eventsGeneric"] {
                    if let Ok(meta) = self.catalog.resolve_display(fallback) {
                        return Some(self.queue_add_node(meta, None));
                    }
                }
                self.result.diagnostics.push(err);
                None
            }
        }
    }

    fn add_call_node(&mut self, call: &Call, target_layer: Option<String>) -> Option<NodeEntity> {
        let meta = match self.catalog.resolve_call(call) {
            Ok(meta) => meta,
            Err(err) => {
                self.result.diagnostics.push(err);
                return None;
            }
        };
        let entity = self.queue_add_node(meta.clone(), target_layer.clone());

        for arg in &call.args {
            let Some(input) = metadata_input_pin(&meta, &arg.name) else {
                self.result.diagnostics.push(format!(
                    "new node `{}` has no input pin named `{}`; skipped that argument",
                    call.display, arg.name
                ));
                continue;
            };

            if let Some(value) = literal_expr_to_value(&arg.value) {
                self.update_commands.push(BoardCommand::UpdateNodePin {
                    node_id: entity.node_ref(),
                    pin_id: input.name.clone(),
                    value,
                    summary: Some(format!("Set {} on {}", input.name, meta.friendly_name)),
                });
                continue;
            }

            let Some(source) = self.resolve_expr(&arg.value, target_layer.clone()) else {
                self.result.diagnostics.push(format!(
                    "argument `{}` on `{}` is not a literal or resolvable node output; skipped connection",
                    arg.name, call.display
                ));
                continue;
            };
            let source = match source {
                SymbolValue::Literal(value) => {
                    self.update_commands.push(BoardCommand::UpdateNodePin {
                        node_id: entity.node_ref(),
                        pin_id: input.name.clone(),
                        value,
                        summary: Some(format!("Set {} on {}", input.name, meta.friendly_name)),
                    });
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
            self.connect_commands.push(BoardCommand::ConnectPins {
                from_node: source.node.node_ref(),
                from_pin: output_pin,
                to_node: entity.node_ref(),
                to_pin: input.name.clone(),
                summary: Some(format!("Connect {} into {}", arg.name, meta.friendly_name)),
            });
        }

        Some(entity)
    }

    fn queue_add_node(&mut self, meta: NodeMetadata, target_layer: Option<String>) -> NodeEntity {
        let ref_id = format!("${}", self.next_ref);
        self.next_ref += 1;
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

    fn connect_exec(
        &mut self,
        previous: &ExecCursor,
        current: &NodeEntity,
        inserted_since_existing: bool,
    ) -> bool {
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
            return false;
        };
        let Some(to_pin) = self.entity_exec_input_pin(current) else {
            return false;
        };

        if matches!(previous.entity, NodeEntity::Existing(_))
            && matches!(current, NodeEntity::Existing(_))
        {
            return false;
        }

        if inserted_since_existing
            && let NodeEntity::Existing(node_id) = current
            && let Some(node) = find_board_node(self.existing, node_id)
        {
            for (from_node, from_pin) in self.board_index.exec_incoming_edges(node, &to_pin) {
                self.disconnect_commands.push(BoardCommand::DisconnectPins {
                    from_node,
                    from_pin,
                    to_node: node_id.clone(),
                    to_pin: to_pin.clone(),
                    summary: Some(format!("Rewire execution into {}", node.friendly_name)),
                });
            }
        }

        self.connect_commands.push(BoardCommand::ConnectPins {
            from_node: previous.entity.node_ref(),
            from_pin,
            to_node: current.node_ref(),
            to_pin,
            summary: Some("Connect FlowScript execution order".to_string()),
        });
        true
    }

    fn entity_exec_input_pin(&self, entity: &NodeEntity) -> Option<String> {
        match entity {
            NodeEntity::Existing(id) => find_board_node(self.existing, id).and_then(exec_input_pin),
            NodeEntity::New { meta, .. } => metadata_exec_input_pin(meta),
        }
    }

    fn entity_exec_output_pin(&self, entity: &NodeEntity) -> Option<String> {
        match entity {
            NodeEntity::Existing(id) => {
                find_board_node(self.existing, id).and_then(exec_output_pin)
            }
            NodeEntity::New { meta, .. } => metadata_exec_output_pin(meta),
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
        }
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
        }
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
                        self.add_variable_get_source(&variable_id, target_layer)?
                    }
                    SymbolValue::Literal(_) => return None,
                };
                let output = self.resolve_entity_output_pin(&source.node, Some(pin))?;
                source.output_pin = Some(output);
                Some(SymbolValue::Source(source))
            }
            Expr::Call(call) => self.add_call_node(call, target_layer).map(|node| {
                SymbolValue::Source(ValueSource {
                    node,
                    output_pin: None,
                })
            }),
            Expr::Member { base, .. } | Expr::Index { base, .. } => {
                self.resolve_expr(base, target_layer)
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
            NodeEntity::Existing(_) => self.resolve_source_output_pin(source),
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
            self.insert_symbol(
                var.name.clone(),
                SymbolValue::VariableRef {
                    variable_id: self.variable_id_for_decl(var),
                },
            );
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
        Stmt::Assign { value, anchor, .. } => anchor.is_none() || expr_has_unanchored_calls(value),
        Stmt::LocalAlias { value, anchor, .. } => {
            anchor.is_none() || expr_has_unanchored_calls(value)
        }
        Stmt::Handler(event) => event.anchor.is_none() || block_has_unanchored_calls(&event.body),
        Stmt::Return { values } => values.iter().any(expr_has_unanchored_calls),
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

fn collect_stmt<'a>(stmt: &'a Stmt, out: &mut HashMap<String, &'a Call>) {
    match stmt {
        Stmt::Let { call, anchor, .. } | Stmt::Call { call, anchor } => {
            collect_call_with_anchor(call, anchor.as_deref(), out)
        }
        Stmt::Branch {
            call, arms, anchor, ..
        } => {
            if !is_placeholder_call(call) {
                collect_call_with_anchor(call, anchor.as_deref(), out);
            }
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
        Stmt::Return { values } => {
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

fn assigned_call_output_pin(expr: &Expr) -> Option<&str> {
    match expr {
        Expr::Field { base, pin } if matches!(base.as_ref(), Expr::Call(_)) => Some(pin),
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
}
