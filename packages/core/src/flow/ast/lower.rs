//! Lowering: `Board -> BoardAst`.
//!
//! Walks exec edges to build ordered statement blocks, inlines pure nodes as expressions,
//! and binds impure-node outputs to `const` names. See `todo/ast.md` §6.

use std::collections::{HashMap, HashSet, VecDeque};

use flow_like_ast::model::*;

use crate::flow::board::{Board, Layer, LayerType};
use crate::flow::node::Node;
use crate::flow::pin::{Pin, PinType};
use crate::flow::variable::{Variable, VariableType};

/// Catalog node type for the pass-through "reroute" node (pure visual wire-bend).
const REROUTE_NODE: &str = "reroute";

/// Variable accessor node types (sugared to bare refs / assignments).
const VARIABLE_GET: &str = "variable_get";
const VARIABLE_SET: &str = "variable_set";

/// Struct node types (sugared to `{…}` literals and `.field` access).
const STRUCT_MAKE: &str = "struct_make";
const STRUCT_MAKE_SCHEMA: &str = "struct_make_from_schema";
const STRUCT_GET: &str = "struct_get";
const STRUCT_BREAK: &str = "struct_break";
const STRUCT_SET: &str = "struct_set";
const STRUCT_SET_IN_PIN: &str = "struct_in";
const STRUCT_SET_OUT_PIN: &str = "struct_out";
const STRUCT_SET_FIELD_PIN: &str = "field";
const STRUCT_SET_VALUE_PIN: &str = "value";

/// Array node types sugared to literals / index / member access.
const MAKE_ARRAY: &str = "make_array";
const ARRAY_GET: &str = "array_get";
const ARRAY_LENGTH: &str = "array_length";

/// Ternary selection node (`condition ? a : b`).
const TYPES_SELECT: &str = "utils_types_select";

/// Generic-event result node sugared to a `return` statement.
const EVENT_RETURN_RESULT: &str = "events_generic_return_result";
const EVENT_RESPONSE_PIN: &str = "response";

/// Dynamic-pin prefixes used by the schema struct nodes.
pub(crate) const MAKE_STRUCT_PREFIX: &str = "__make_struct_field__";
pub(crate) const BREAK_STRUCT_PREFIX: &str = "__break_struct_field__";

/// Loop node types and their FlowScript keyword. Each loops its `exec_out` exec output as the
/// body and continues the enclosing chain from `done`.
const LOOP_NODES: &[(&str, &str)] = &[
    ("control_for_each", "forEach"),
    ("control_par_for_each", "forEachParallel"),
    ("control_while_loop", "while"),
];

/// Exec output pin opening a loop's body.
const LOOP_BODY_PIN: &str = "exec_out";
/// Exec output pin continuing the chain once a loop finishes.
const LOOP_DONE_PIN: &str = "done";

/// Node that calls another node by id (`fn_ref` holds an opaque target node id).
const CALL_REFERENCE: &str = "control_call_reference";
const FN_REF_PIN: &str = "fn_ref";

/// Node that calls a function layer by id (`function_layer_id` holds the target layer id).
const CALL_FUNCTION: &str = "control_call_function";
const FUNCTION_LAYER_ID_PIN: &str = "function_layer_id";

/// Conditional branch node and its boolean condition input pin.
const CONTROL_BRANCH: &str = "control_branch";
const CONDITION_PIN: &str = "condition";

/// Agent nodes that register Flow-Like functions as callable tools (`fn_refs` holds the
/// referenced tool entry-node ids). These surface their references under a `tools:` argument.
const AGENT_REGISTER_TOOLS: &[&str] = &[
    "agent_register_function_tools",
    "agent_lazy_register_function_tools",
];

/// Synthetic argument name carrying an agent's registered tool references.
const TOOLS_ARG: &str = "tools";
/// Synthetic argument name carrying a node's generic function references.
const FN_REFS_ARG: &str = "fnRefs";

/// Comparison/arithmetic/logic nodes sugared to a binary `lhs <op> rhs` expression. Only applied
/// when the node exposes exactly two data inputs; operands are read by pin index (their names are
/// inconsistent across the int/float/string/bool families, e.g. `integer1`/`base`/`string`).
const BINARY_OPS: &[(&str, &str)] = &[
    // Integer comparison + arithmetic.
    ("int_equal", "=="),
    ("int_unequal", "!="),
    ("int_greater_than", ">"),
    ("int_greater_than_or_equal", ">="),
    ("int_less_than", "<"),
    ("int_less_than_or_equal", "<="),
    ("int_add", "+"),
    ("int_subtract", "-"),
    ("int_multiply", "*"),
    ("int_divide", "/"),
    ("int_modulo", "%"),
    ("int_power", "**"),
    // Float comparison + arithmetic.
    ("float_equal", "=="),
    ("float_unequal", "!="),
    ("float_greater_than", ">"),
    ("float_greater_than_or_equal", ">="),
    ("float_less_than", "<"),
    ("float_less_than_or_equal", "<="),
    ("float_add", "+"),
    ("float_subtract", "-"),
    ("float_multiply", "*"),
    ("float_divide", "/"),
    ("float_power", "**"),
    // String equality.
    ("equal_string", "=="),
    ("not_equal_string", "!="),
    // Boolean logic.
    ("bool_equal", "=="),
    ("bool_and", "&&"),
    ("bool_or", "||"),
    ("bool_xor", "^"),
];

/// If `node` is a sugarable binary-operator node, return its JS operator.
fn binary_op(node: &Node) -> Option<&'static str> {
    BINARY_OPS
        .iter()
        .find(|(ty, _)| *ty == node.name)
        .map(|(_, op)| *op)
}

/// If `node` is a loop, return its FlowScript keyword.
fn loop_keyword(node: &Node) -> Option<&'static str> {
    LOOP_NODES
        .iter()
        .find(|(ty, _)| *ty == node.name)
        .map(|(_, kw)| *kw)
}

/// Board-side mapping helpers (depend on core's `VariableType`/`ValueType`).
mod util {
    use flow_like_ast::model::Literal;
    pub use flow_like_ast::to_camel_case;

    pub(crate) use super::super::types::type_ref;

    /// Decode a pin/variable JSON default (`Vec<u8>`) into an AST `Literal`.
    pub fn decode_default(bytes: &[u8]) -> Option<Literal> {
        let value = decode_value(bytes)?;
        literal_from_value(&value)
    }

    /// Decode a configured JSON default while preserving an explicit `null` value.
    pub fn decode_default_preserving_null(bytes: &[u8]) -> Option<Literal> {
        let value = decode_value(bytes)?;
        match value {
            flow_like_types::Value::Null => Some(Literal::Null),
            other => literal_from_value(&other),
        }
    }

    fn decode_value(bytes: &[u8]) -> Option<flow_like_types::Value> {
        if bytes.is_empty() {
            return None;
        }
        flow_like_types::json::from_slice(bytes).ok()
    }

    pub fn literal_from_value(value: &flow_like_types::Value) -> Option<Literal> {
        use flow_like_types::Value;
        match value {
            Value::Null => None,
            Value::Bool(b) => Some(Literal::Bool(*b)),
            Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    Some(Literal::Int(i))
                } else {
                    n.as_f64().map(Literal::Float)
                }
            }
            Value::String(s) => Some(Literal::String(s.clone())),
            other => flow_like_types::json::to_string(other)
                .ok()
                .map(Literal::Json),
        }
    }
}

/// Lower a whole board into the FlowScript AST.
pub fn lower_board(board: &Board) -> BoardAst {
    let mut lowering = Lowering::new(board);
    lowering.run()
}

#[derive(Clone, Copy)]
struct CanonicalBoardNode<'a> {
    node: &'a Node,
    /// Effective semantic scope. Canonical flat nodes carry it on `node.layer`; legacy
    /// layer-local-only nodes inherit it from the map that contains them.
    layer: Option<&'a str>,
}

/// Return one semantic node per id across the canonical flat store and the legacy layer-local
/// compatibility maps. Flat `board.nodes` entries are authoritative when both representations are
/// present; a layer-local entry only fills an id missing from the flat store.
fn canonical_board_nodes<'a>(board: &'a Board) -> Vec<CanonicalBoardNode<'a>> {
    let mut by_id: HashMap<&'a str, CanonicalBoardNode<'a>> = HashMap::new();

    // A malformed/readback board can retain the same legacy-only identity in more than one
    // layer-local map. Pick the first layer/node in stable id order so lowering does not depend on
    // HashMap iteration order. Canonical flat entries below still replace every legacy fallback.
    let mut legacy_layers = board.layers.iter().collect::<Vec<_>>();
    legacy_layers.sort_by(|(left_key, left), (right_key, right)| {
        left.id.cmp(&right.id).then_with(|| left_key.cmp(right_key))
    });
    for (_, layer) in legacy_layers {
        let mut legacy_nodes = layer.nodes.iter().collect::<Vec<_>>();
        legacy_nodes.sort_by(|(left_key, left), (right_key, right)| {
            left.id.cmp(&right.id).then_with(|| left_key.cmp(right_key))
        });
        for (_, node) in legacy_nodes {
            let effective_layer = node
                .layer
                .as_deref()
                .filter(|layer_id| !layer_id.is_empty())
                .or(Some(layer.id.as_str()));
            by_id.entry(node.id.as_str()).or_insert(CanonicalBoardNode {
                node,
                layer: effective_layer,
            });
        }
    }
    for node in board.nodes.values() {
        by_id.insert(
            node.id.as_str(),
            CanonicalBoardNode {
                node,
                layer: node
                    .layer
                    .as_deref()
                    .filter(|layer_id| !layer_id.is_empty()),
            },
        );
    }

    let mut nodes = by_id.into_values().collect::<Vec<_>>();
    nodes.sort_by(|left, right| left.node.id.cmp(&right.node.id));
    nodes
}

struct Lowering<'a> {
    board: &'a Board,
    /// Interfaces derived from every schema-bearing text surface (variables, Function boundary
    /// pins, and event outputs), built before lowering so Params can use stable nominal names.
    interfaces: Vec<InterfaceDecl>,
    /// pin id -> owning node (across the whole board, all scopes).
    pin_owner: HashMap<&'a str, &'a Node>,
    /// pin id -> pin (across the whole board).
    pins: HashMap<&'a str, &'a Pin>,
    /// node id -> node (across the whole board, all scopes).
    nodes_by_id: HashMap<&'a str, &'a Node>,
    /// function-layer boundary input pin id -> camelCase parameter name.
    boundary_params: HashMap<&'a str, String>,
    /// presentational (collapsed/macro) layer boundary pin id -> the boundary pin. These bridge
    /// exec/data edges across a sub-layer's frame; resolution follows them transparently so the
    /// graph reads as if the layer were inlined.
    boundary_pins: HashMap<&'a str, &'a Pin>,
    /// event-entry data output pin id -> camelCase event parameter name (payload fields surfaced
    /// as the event's typed parameter list, resolved to bare references in the body).
    event_params: HashMap<&'a str, String>,
    /// variable id -> camelCase variable name (for `varRef` sugar).
    var_names: HashMap<&'a str, String>,
    /// function-layer id -> camelCase function name (for `fnRef`/`functionLayerId` sugar).
    fn_names: HashMap<&'a str, String>,
    /// node id -> stable `const` binding name (impure nodes with consumed outputs).
    bindings: HashMap<String, String>,
    /// node id -> readable accumulator name for `structSet(...).structOut` update chains.
    struct_accumulators: HashMap<String, String>,
    /// node ids already emitted during the current exec walk.
    visited: HashSet<String>,
    /// guard against cycles while inlining pure expressions.
    inlining: HashSet<String>,
}

impl<'a> Lowering<'a> {
    fn new(board: &'a Board) -> Self {
        let interfaces = interfaces_for_board_text_surfaces(board);
        let mut pin_owner = HashMap::new();
        let mut pins = HashMap::new();
        let mut nodes_by_id = HashMap::new();
        let mut index = |node: &'a Node| {
            nodes_by_id.insert(node.id.as_str(), node);
            for pin in node.pins.values() {
                pin_owner.insert(pin.id.as_str(), node);
                pins.insert(pin.id.as_str(), pin);
            }
        };
        for indexed in canonical_board_nodes(board) {
            index(indexed.node);
        }

        // Map function-layer boundary input pins to their parameter names so nodes inside the
        // function that read from the boundary render as `param` references.
        let mut boundary_params = HashMap::new();
        for layer in board.layers.values() {
            if !matches!(layer.r#type, LayerType::Function) {
                continue;
            }
            for pin in layer.pins.values() {
                if pin.pin_type == PinType::Input && pin.data_type != VariableType::Execution {
                    boundary_params.insert(pin.id.as_str(), util::to_camel_case(&pin.name));
                }
            }
        }

        // Index presentational (collapsed/macro) layer boundary pins so exec/data edges crossing
        // a sub-layer frame can be followed transparently. Function layers use the parameter
        // mechanism above and are called rather than exec-bridged, so they are excluded.
        let mut boundary_pins = HashMap::new();
        for layer in board.layers.values() {
            if matches!(layer.r#type, LayerType::Function) {
                continue;
            }
            for pin in layer.pins.values() {
                boundary_pins.insert(pin.id.as_str(), pin);
            }
        }

        // Resolve opaque ids to human names: variable ids -> camelCase variable names, and
        // function-layer ids -> camelCase function names. These back the `varRef` / `fnRef`
        // sugar so the clean text never leaks a CUID.
        let mut var_names = HashMap::new();
        for var in board.variables.values() {
            var_names.insert(var.id.as_str(), util::to_camel_case(&var.name));
        }
        // Layer-local variables (e.g. function-scoped) also back `varRef` sugar.
        for layer in board.layers.values() {
            for var in layer.variables.values() {
                var_names
                    .entry(var.id.as_str())
                    .or_insert_with(|| util::to_camel_case(&var.name));
            }
        }
        let mut fn_names = HashMap::new();
        for layer in board.layers.values() {
            if matches!(layer.r#type, LayerType::Function) {
                fn_names.insert(layer.id.as_str(), util::to_camel_case(&layer.name));
            }
        }

        Self {
            board,
            interfaces,
            pin_owner,
            pins,
            nodes_by_id,
            boundary_params,
            boundary_pins,
            event_params: HashMap::new(),
            var_names,
            fn_names,
            bindings: HashMap::new(),
            struct_accumulators: HashMap::new(),
            visited: HashSet::new(),
            inlining: HashSet::new(),
        }
    }

    fn run(&mut self) -> BoardAst {
        self.assign_bindings();
        self.assign_struct_accumulators();

        let variables = lower_variables(self.board.variables.values(), &self.board.refs);

        // Function layers become `function` declarations, scoped to their member nodes.
        let function_ids: HashSet<&str> = self
            .board
            .layers
            .values()
            .filter(|l| matches!(l.r#type, LayerType::Function))
            .map(|l| l.id.as_str())
            .collect();

        // Resolve every layer to its nearest enclosing function layer (itself if it is one).
        // Presentational sub-layers (collapsed/macro) nested under a function belong to that
        // function's scope — e.g. an agent tool's body graph lives in a child layer of the
        // function that declares the tool. Layers with no function ancestor flatten into root.
        let layer_parent: HashMap<&str, Option<&str>> = self
            .board
            .layers
            .values()
            .map(|l| (l.id.as_str(), l.parent_id.as_deref()))
            .collect();
        let mut owning_function: HashMap<&str, &str> = HashMap::new();
        for layer in self.board.layers.values() {
            let mut cur = layer.id.as_str();
            let mut seen = HashSet::new();
            loop {
                // Corrupt/self-referential presentational ancestry must not hang FlowScript
                // lowering (and therefore every reconcile, which lowers the existing board).
                if !seen.insert(cur) {
                    break;
                }
                if function_ids.contains(cur) {
                    owning_function.insert(layer.id.as_str(), cur);
                    break;
                }
                match layer_parent.get(cur).copied().flatten() {
                    Some(parent) => cur = parent,
                    None => break,
                }
            }
        }

        // Group nodes by the function they ultimately belong to (or root).
        let mut function_nodes: HashMap<&str, Vec<&Node>> = HashMap::new();
        let mut root_nodes: Vec<&Node> = Vec::new();
        for indexed in canonical_board_nodes(self.board) {
            match indexed
                .layer
                .and_then(|layer_id| owning_function.get(layer_id).copied())
            {
                Some(fid) => function_nodes.entry(fid).or_default().push(indexed.node),
                None => root_nodes.push(indexed.node),
            }
        }

        let mut functions = Vec::new();
        let mut function_layers: Vec<&Layer> = self
            .board
            .layers
            .values()
            .filter(|l| matches!(l.r#type, LayerType::Function))
            .collect();
        function_layers.sort_by(|a, b| a.id.cmp(&b.id));
        for layer in function_layers {
            let nodes = function_nodes
                .get(layer.id.as_str())
                .cloned()
                .unwrap_or_default();
            functions.push(self.lower_function(layer, &nodes));
        }

        let events = self.lower_events(&root_nodes);

        BoardAst {
            board_id: self.board.id.clone(),
            interfaces: self.interfaces.clone(),
            variables,
            functions,
            events,
        }
    }

    /// Pre-pass: assign a stable `const` name to every impure node whose data output is
    /// consumed by another node.
    fn assign_bindings(&mut self) {
        let mut used_names: HashSet<String> = HashSet::new();
        // Deterministic order for stable name suffixes.
        let mut nodes: Vec<&Node> = self.pin_owner.values().copied().collect();
        nodes.sort_by(|a, b| a.id.cmp(&b.id));
        nodes.dedup_by(|a, b| a.id == b.id);

        for node in nodes {
            if !is_impure(node) {
                continue;
            }
            let produces_consumed_output = node.pins.values().any(|p| {
                p.pin_type == PinType::Output && !is_exec(p) && !p.connected_to.is_empty()
            });
            if !produces_consumed_output {
                continue;
            }
            let base = binding_base_name(node);
            let name = unique_name(&base, &mut used_names);
            self.bindings.insert(node.id.clone(), name);
        }
    }

    /// Pre-pass: assign one readable mutable alias to chains of `structSet` nodes.
    ///
    /// Without this, a record assembled with repeated struct updates renders as
    /// `setField3.structOut -> setField5.structOut -> ...`. The accumulator keeps every node
    /// visible and anchored while making the data-flow shape much easier to continue editing:
    /// `row = structSet({ structIn: row, ... }).structOut`.
    fn assign_struct_accumulators(&mut self) {
        let mut used_names: HashSet<String> = self.bindings.values().cloned().collect();
        let mut nodes: Vec<&Node> = self.nodes_by_id.values().copied().collect();
        nodes.sort_by(|a, b| a.id.cmp(&b.id));
        nodes.dedup_by(|a, b| a.id == b.id);

        for node in nodes {
            if node.name != STRUCT_SET
                || self.struct_accumulators.contains_key(&node.id)
                || self.previous_struct_set(node).is_some()
            {
                continue;
            }

            let name = unique_name(self.struct_accumulator_base_name(node), &mut used_names);
            let mut current = Some(node);
            while let Some(struct_set) = current {
                self.struct_accumulators
                    .insert(struct_set.id.clone(), name.clone());
                current = self.next_struct_set(struct_set);
            }
        }
    }

    fn struct_accumulator_base_name(&self, start: &'a Node) -> &'static str {
        let mut current = start;
        while let Some(next) = self.next_struct_set(current) {
            current = next;
        }

        for out_pin in current
            .pins
            .values()
            .filter(|pin| pin.pin_type == PinType::Output && !is_exec(pin))
        {
            for target_pin in self.downstream_pins(out_pin) {
                let Some(target_node) = self.pin_owner.get(target_pin.id.as_str()).copied() else {
                    continue;
                };
                if target_pin.name == "value"
                    || target_node.name.contains("local_db")
                    || target_node.name.contains("insert")
                    || target_node.name.contains("upsert")
                {
                    return "row";
                }
            }
        }
        "record"
    }

    fn previous_struct_set(&self, node: &'a Node) -> Option<&'a Node> {
        let input = node
            .pins
            .values()
            .find(|p| p.pin_type == PinType::Input && !is_exec(p) && p.name == STRUCT_SET_IN_PIN)?;
        let source_pin_id = input.depends_on.iter().next()?;
        let source_node = *self.pin_owner.get(source_pin_id.as_str())?;
        (source_node.name == STRUCT_SET).then_some(source_node)
    }

    fn next_struct_set(&self, node: &'a Node) -> Option<&'a Node> {
        let output = self.struct_set_output_pin(node)?;
        let mut next = None;
        for target_pin in self.downstream_pins(output) {
            let Some(target_node) = self.pin_owner.get(target_pin.id.as_str()).copied() else {
                continue;
            };
            if target_node.name == STRUCT_SET
                && target_pin.pin_type == PinType::Input
                && target_pin.name == STRUCT_SET_IN_PIN
            {
                if next.is_some() {
                    return None;
                }
                next = Some(target_node);
            }
        }
        next
    }

    fn downstream_pins(&self, source: &'a Pin) -> Vec<&'a Pin> {
        let mut pins = Vec::new();
        let mut seen: HashSet<&str> = HashSet::new();

        for target_pin_id in &source.connected_to {
            if seen.insert(target_pin_id.as_str())
                && let Some(pin) = self.pins.get(target_pin_id.as_str()).copied() {
                    pins.push(pin);
                }
        }

        for pin in self.pins.values().copied() {
            if pin.depends_on.contains(source.id.as_str()) && seen.insert(pin.id.as_str()) {
                pins.push(pin);
            }
        }

        pins
    }

    fn struct_set_output_pin(&self, node: &'a Node) -> Option<&'a Pin> {
        node.pins
            .values()
            .find(|p| p.pin_type == PinType::Output && !is_exec(p) && p.name == STRUCT_SET_OUT_PIN)
            .or_else(|| {
                let mut outputs: Vec<&Pin> = node
                    .pins
                    .values()
                    .filter(|p| p.pin_type == PinType::Output && !is_exec(p))
                    .collect();
                outputs.sort_by_key(|p| p.index);
                outputs.into_iter().next()
            })
    }

    fn lower_function(&mut self, layer: &'a Layer, nodes: &[&'a Node]) -> FnDecl {
        let mut params = Vec::new();
        let mut returns = Vec::new();
        let mut boundary: Vec<&Pin> = layer.pins.values().filter(|p| !is_exec(p)).collect();
        boundary.sort_by_key(|p| p.index);
        for pin in boundary {
            let param = Param {
                name: util::to_camel_case(&pin.name),
                ty: self.type_ref_for_pin(pin),
            };
            match pin.pin_type {
                PinType::Input => params.push(param),
                PinType::Output => returns.push(param),
            }
        }

        let mut body = self.lower_scope_body(nodes);

        let fn_name = util::to_camel_case(&layer.name);
        let (return_stmt, folded_return_variables) =
            self.lower_function_return(layer, nodes, &fn_name);

        // Prepend the function's own (layer-local) variables as `let` declarations so the body
        // can read/assign them without leaking an undeclared identifier. Variables materialized
        // for a literal `return` are folded into that statement instead of an inert decl.
        let mut locals: Vec<&Variable> = layer
            .variables
            .values()
            .filter(|v| !folded_return_variables.contains(v.id.as_str()))
            .collect();
        locals.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
        let local_stmts: Vec<Stmt> = locals
            .iter()
            .map(|v| Stmt::Local(var_decl_of(v, &self.board.refs)))
            .collect();
        body.stmts.splice(0..0, local_stmts);
        body.stmts.extend(return_stmt);

        FnDecl {
            name: fn_name,
            params,
            returns,
            body,
            anchor: Some(layer.id.clone()),
        }
    }

    /// Render the boundary return wiring as a trailing `return` statement when every (non-exec)
    /// return pin has one resolvable data source, so the statement survives the lower→reconcile
    /// round-trip instead of being re-added (and re-materialized) by the model on every turn.
    /// A variable materialized for a literal return (`{fn}_{pin}` name, stored default, no other
    /// consumers) renders as its literal and its id is returned for local-decl suppression; any
    /// other variable renders as a bare reference; every other producer renders through the same
    /// expression resolution the body uses (`hashed.hash`, `user.hasUser`, member chains).
    /// Anything unresolvable keeps the status quo (no statement).
    fn lower_function_return(
        &mut self,
        layer: &'a Layer,
        nodes: &[&'a Node],
        fn_name: &str,
    ) -> (Option<Stmt>, HashSet<&'a str>) {
        let mut return_pins: Vec<&Pin> = layer
            .pins
            .values()
            .filter(|p| p.pin_type == PinType::Output && !is_exec(p))
            .collect();
        return_pins.sort_by_key(|p| p.index);
        if return_pins.is_empty() {
            return (None, HashSet::new());
        }

        let mut values = Vec::new();
        let mut folded: HashSet<&'a str> = HashSet::new();
        for pin in return_pins {
            let mut sources = pin.depends_on.iter();
            let (Some(source_pin_id), None) = (sources.next(), sources.next()) else {
                return (None, HashSet::new());
            };
            let Some(owner) = self.pin_owner.get(source_pin_id.as_str()).copied() else {
                return (None, HashSet::new());
            };
            if owner.name != VARIABLE_GET {
                match self.resolve_source(source_pin_id) {
                    Some(expr) => values.push(expr),
                    None => return (None, HashSet::new()),
                }
                continue;
            }
            let Some(variable_id) = self.pin_literal_string(owner, "var_ref") else {
                return (None, HashSet::new());
            };

            let variable = layer.variables.get(&variable_id);
            let literal = variable
                .filter(|v| is_materialized_return_name(&v.name, fn_name, &pin.name))
                .filter(|v| self.variable_only_feeds_layer_boundary(v, nodes, layer))
                .and_then(|v| v.default_value.as_deref().and_then(util::decode_default));
            if let (Some(variable), Some(literal)) = (variable, literal) {
                folded.insert(variable.id.as_str());
                values.push(Expr::Literal(literal));
                continue;
            }
            match self.var_names.get(variable_id.as_str()) {
                Some(name) => values.push(Expr::Ref(name.clone())),
                None => return (None, HashSet::new()),
            }
        }
        (
            Some(Stmt::Return {
                values,
                anchor: None,
            }),
            folded,
        )
    }

    /// A materialized literal-return variable may be folded into its `return <literal>` rendering
    /// only when nothing else consumes it: every node referencing it is a `variable_get` whose
    /// data outputs feed this layer's boundary Output pins exclusively.
    fn variable_only_feeds_layer_boundary(
        &self,
        variable: &Variable,
        nodes: &[&'a Node],
        layer: &'a Layer,
    ) -> bool {
        let boundary_output_ids: HashSet<&str> = layer
            .pins
            .values()
            .filter(|p| p.pin_type == PinType::Output && !is_exec(p))
            .map(|p| p.id.as_str())
            .collect();
        for node in nodes {
            let references_variable = self
                .pin_literal_string(node, "var_ref")
                .is_some_and(|id| id == variable.id);
            if !references_variable {
                continue;
            }
            if node.name != VARIABLE_GET {
                return false;
            }
            for output in node
                .pins
                .values()
                .filter(|p| p.pin_type == PinType::Output && !is_exec(p))
            {
                if !output
                    .connected_to
                    .iter()
                    .all(|target| boundary_output_ids.contains(target.as_str()))
                {
                    return false;
                }
                if self
                    .pins
                    .values()
                    .any(|pin| pin.depends_on.contains(output.id.as_str()))
                {
                    return false;
                }
            }
        }
        true
    }

    fn lower_events(&mut self, scope: &[&'a Node]) -> Vec<EventBlock> {
        let scope_ids: HashSet<&str> = scope.iter().map(|n| n.id.as_str()).collect();
        let mut entries: Vec<&Node> = scope
            .iter()
            .copied()
            .filter(|n| is_impure(n) && self.is_scope_entry(n, &scope_ids))
            .collect();
        entries.sort_by(|a, b| entry_order(a).cmp(&entry_order(b)).then(a.id.cmp(&b.id)));

        // First pass: surface each event's data output pins as a typed parameter list and register
        // the pin -> bare parameter mapping so body references resolve to the declared names rather
        // than `eventName.field`.
        let mut params_by_entry: HashMap<&str, Vec<Param>> = HashMap::new();
        for entry in &entries {
            if self.visited.contains(&entry.id) {
                continue;
            }
            params_by_entry.insert(entry.id.as_str(), self.event_params_of(entry));
        }

        let mut events = Vec::new();
        for entry in entries {
            if self.visited.contains(&entry.id) {
                continue;
            }
            let params = params_by_entry
                .remove(entry.id.as_str())
                .unwrap_or_default();
            let body = self.walk_entry_body(entry);
            events.push(EventBlock {
                name: event_type_name(entry),
                node_type: entry.name.clone(),
                event_name: event_alias(entry),
                params,
                body,
                anchor: Some(entry.id.clone()),
            });
        }
        self.nest_root_agent_tool_handlers(events)
    }

    /// Root-level tool entry nodes are stored beside their owning app event on the board, even
    /// though FlowScript gives them lexical ownership by nesting them in that event's body. The
    /// registration node plus a concrete cross-entry data edge is the authoritative relationship:
    /// when one root event body registers a referenceable root entry and that handler consumes an
    /// output of the event, move the entry under the event as a `Stmt::Handler`.
    ///
    /// Do this only for a unique capturing owner. An entry that captures outputs from multiple
    /// registering root events (or is involved in a malformed ownership cycle) remains top-level
    /// rather than guessing a scope and silently changing what a bare reference resolves to.
    fn nest_root_agent_tool_handlers(&self, events: Vec<EventBlock>) -> Vec<EventBlock> {
        if events.len() < 2 {
            return events;
        }

        let event_index_by_anchor: HashMap<&str, usize> = events
            .iter()
            .enumerate()
            .filter_map(|(index, event)| event.anchor.as_deref().map(|anchor| (anchor, index)))
            .collect();
        let mut owners_by_handler: HashMap<usize, HashSet<usize>> = HashMap::new();

        for (owner_index, event) in events.iter().enumerate() {
            let mut calls = Vec::new();
            collect_calls_in_block(&event.body, &mut calls);
            for call in calls {
                let Some(register) = call
                    .anchor
                    .as_deref()
                    .and_then(|anchor| self.nodes_by_id.get(anchor).copied())
                else {
                    continue;
                };
                if !AGENT_REGISTER_TOOLS.contains(&register.name.as_str()) {
                    continue;
                }
                let Some(fn_refs) = register
                    .fn_refs
                    .as_ref()
                    .filter(|refs| refs.can_reference_fns)
                else {
                    continue;
                };

                for target_id in &fn_refs.fn_refs {
                    let Some(&handler_index) = event_index_by_anchor.get(target_id.as_str()) else {
                        continue;
                    };
                    if handler_index == owner_index {
                        continue;
                    }
                    let is_referenceable_entry = self
                        .nodes_by_id
                        .get(target_id.as_str())
                        .and_then(|target| target.fn_refs.as_ref())
                        .is_some_and(|refs| refs.can_be_referenced_by_fns);
                    if !is_referenceable_entry {
                        continue;
                    }
                    let Some(owner_id) = event.anchor.as_deref() else {
                        continue;
                    };
                    // A nested handler parameter would shadow the owner's same-named capture.
                    // Keep that still-ambiguous shape top-level until FlowScript can qualify
                    // cross-handler sources explicitly.
                    if event.params.iter().any(|owner_param| {
                        events[handler_index]
                            .params
                            .iter()
                            .any(|handler_param| handler_param.name == owner_param.name)
                    }) {
                        continue;
                    }
                    if !self.handler_captures_event_output(&events[handler_index], owner_id) {
                        continue;
                    }
                    owners_by_handler
                        .entry(handler_index)
                        .or_default()
                        .insert(owner_index);
                }
            }
        }

        let mut parent_by_child: HashMap<usize, usize> = owners_by_handler
            .into_iter()
            .filter_map(|(child, owners)| {
                (owners.len() == 1).then(|| (child, *owners.iter().next().expect("one owner")))
            })
            .collect();

        // Reject every ownership path that reaches a cycle. This also keeps descendants of a
        // malformed cycle at the root instead of partially nesting an unsafe hierarchy.
        let cyclic_paths: HashSet<usize> = parent_by_child
            .keys()
            .copied()
            .filter(|start| {
                let mut seen = HashSet::new();
                let mut current = *start;
                loop {
                    if !seen.insert(current) {
                        return true;
                    }
                    let Some(parent) = parent_by_child.get(&current).copied() else {
                        return false;
                    };
                    current = parent;
                }
            })
            .collect();
        parent_by_child.retain(|child, parent| {
            !cyclic_paths.contains(child) && !cyclic_paths.contains(parent)
        });

        let mut children_by_parent: HashMap<usize, Vec<usize>> = HashMap::new();
        for (&child, &parent) in &parent_by_child {
            children_by_parent.entry(parent).or_default().push(child);
        }
        for children in children_by_parent.values_mut() {
            children.sort_unstable();
        }

        fn assemble_event(
            index: usize,
            slots: &mut [Option<EventBlock>],
            children_by_parent: &HashMap<usize, Vec<usize>>,
        ) -> EventBlock {
            let mut event = slots[index]
                .take()
                .expect("each lowered event is assembled exactly once");
            if let Some(children) = children_by_parent.get(&index) {
                for child in children {
                    event.body.stmts.push(Stmt::Handler(assemble_event(
                        *child,
                        slots,
                        children_by_parent,
                    )));
                }
            }
            event
        }

        let mut slots: Vec<Option<EventBlock>> = events.into_iter().map(Some).collect();
        let mut nested = Vec::new();
        for index in 0..slots.len() {
            if !parent_by_child.contains_key(&index) {
                nested.push(assemble_event(index, &mut slots, &children_by_parent));
            }
        }
        // Defensive fallback for malformed ownership metadata not covered above: never drop an
        // event from rendered FlowScript.
        nested.extend(slots.into_iter().flatten());
        nested
    }

    /// Whether an emitted handler body has a data dependency on one of `owner_id`'s outputs.
    /// That concrete cross-entry edge is the evidence that the handler needs the registering
    /// event's lexical parameter scope; registration alone is not enough because the same tool
    /// entry may intentionally be shared by independent events.
    fn handler_captures_event_output(&self, handler: &EventBlock, owner_id: &str) -> bool {
        let mut calls = Vec::new();
        collect_calls_in_block(&handler.body, &mut calls);
        calls.into_iter().any(|call| {
            let Some(node) = call
                .anchor
                .as_deref()
                .and_then(|anchor| self.nodes_by_id.get(anchor).copied())
            else {
                return false;
            };
            node.pins
                .values()
                .filter(|pin| pin.pin_type == PinType::Input && !is_exec(pin))
                .flat_map(|pin| &pin.depends_on)
                .any(|source_pin_id| {
                    self.data_source_reaches_node(source_pin_id, owner_id, &mut HashSet::new())
                })
        })
    }

    /// Follow upstream data plumbing from one source pin. Pure transforms and reroutes still
    /// carry the owner event's lexical value, so recurse through every non-exec input while
    /// guarding malformed graph cycles by pin id.
    fn data_source_reaches_node(
        &self,
        source_pin_id: &str,
        target_node_id: &str,
        seen: &mut HashSet<String>,
    ) -> bool {
        if !seen.insert(source_pin_id.to_string()) {
            return false;
        }
        if let Some(source_node) = self.pin_owner.get(source_pin_id).copied() {
            if source_node.id == target_node_id {
                return true;
            }
            return source_node
                .pins
                .values()
                .filter(|pin| pin.pin_type == PinType::Input && !is_exec(pin))
                .flat_map(|pin| &pin.depends_on)
                .any(|upstream| self.data_source_reaches_node(upstream, target_node_id, seen));
        }
        self.boundary_pins
            .get(source_pin_id)
            .is_some_and(|boundary| {
                boundary
                    .depends_on
                    .iter()
                    .any(|upstream| self.data_source_reaches_node(upstream, target_node_id, seen))
            })
    }

    /// Collect an event entry's non-exec data output pins as a typed parameter list, registering
    /// each pin id under a unique camelCase name in `event_params`.
    fn event_params_of(&mut self, entry: &'a Node) -> Vec<Param> {
        let mut outputs: Vec<&Pin> = entry
            .pins
            .values()
            .filter(|p| p.pin_type == PinType::Output && !is_exec(p))
            .collect();
        outputs.sort_by_key(|p| p.index);

        let mut used: HashSet<String> = HashSet::new();
        let mut params = Vec::new();
        for pin in outputs {
            let name = unique_name(&util::to_camel_case(&pin.name), &mut used);
            self.event_params.insert(pin.id.as_str(), name.clone());
            params.push(Param {
                name,
                ty: self.type_ref_for_pin(pin),
            });
        }
        params
    }

    fn type_ref_for_pin(&self, pin: &Pin) -> TypeRef {
        let mut ty = util::type_ref(&pin.data_type, &pin.value_type);
        if pin.data_type != VariableType::Struct {
            return ty;
        }
        if pin
            .options
            .as_ref()
            .and_then(|options| options.enforce_schema)
            != Some(true)
        {
            return ty;
        }
        let Some(raw_schema) = pin.schema.as_deref() else {
            return ty;
        };
        let schema = self
            .board
            .refs
            .get(raw_schema)
            .map(String::as_str)
            .unwrap_or(raw_schema);
        if let Some(interface_name) =
            flow_like_ast::interface_name_for_schema(&self.interfaces, schema)
        {
            ty.base = interface_name.to_string();
        }
        ty
    }

    /// Lower a function-layer body: walk every entry, then return outputs.
    fn lower_scope_body(&mut self, scope: &[&'a Node]) -> Block {
        let scope_ids: HashSet<&str> = scope.iter().map(|n| n.id.as_str()).collect();
        let mut entries: Vec<&Node> = scope
            .iter()
            .copied()
            .filter(|n| is_impure(n) && self.is_scope_entry(n, &scope_ids))
            .collect();
        entries.sort_by(|a, b| entry_order(a).cmp(&entry_order(b)).then(a.id.cmp(&b.id)));

        // Trigger entries (`start`/`event_callback` nodes — e.g. agent tool entry points) are
        // independent handlers that close over the scope's locals, not part of its linear flow.
        // Pre-register their payload outputs as parameters so their bodies resolve to bare names.
        let mut params_by_entry: HashMap<&str, Vec<Param>> = HashMap::new();
        for entry in &entries {
            if is_trigger_entry(entry) && !self.visited.contains(&entry.id) {
                params_by_entry.insert(entry.id.as_str(), self.event_params_of(entry));
            }
        }

        let mut block = Block::default();
        for entry in entries {
            if self.visited.contains(&entry.id) {
                continue;
            }
            if is_trigger_entry(entry) {
                // Render as a nested event handler (`name(params) { … }`).
                let params = params_by_entry
                    .remove(entry.id.as_str())
                    .unwrap_or_default();
                let body = self.walk_entry_body(entry);
                block.stmts.push(Stmt::Handler(EventBlock {
                    name: event_type_name(entry),
                    node_type: entry.name.clone(),
                    event_name: event_alias(entry),
                    params,
                    body,
                    anchor: Some(entry.id.clone()),
                }));
            } else {
                // A function scope has no trigger: the entry node is a real statement, so emit
                // it (and its chain) directly.
                let mut walked = self.walk_from(&entry.id);
                block.stmts.append(&mut walked.stmts);
            }
        }
        block
    }

    /// Body of an exec entry (event or function entry): the entry node itself becomes the
    /// block header, so its body is the chain that follows its exec output(s).
    fn walk_entry_body(&mut self, entry: &'a Node) -> Block {
        self.visited.insert(entry.id.clone());
        let exec_outs = exec_output_pins(entry);

        if exec_outs.len() <= 1 {
            match exec_outs.first().and_then(|p| self.first_exec_target(p)) {
                Some(target) => self.walk_from(&target),
                None => Block::default(),
            }
        } else {
            let mut block = Block::default();
            let (arms, join) = self.lower_exec_arms(entry, &exec_outs, None);
            block.stmts.push(Stmt::Branch {
                bind: None,
                call: self.build_call(entry),
                condition: None,
                arms,
                anchor: Some(entry.id.clone()),
            });
            if let Some(join) = join {
                let mut continuation = self.walk_from(&join);
                block.stmts.append(&mut continuation.stmts);
            }
            block
        }
    }

    /// Follow the linear exec chain from `node_id`, opening nested blocks at branch nodes.
    fn walk_from(&mut self, node_id: &str) -> Block {
        self.walk_from_until(node_id, None)
    }

    /// Follow an exec chain without consuming `stop_before`. Branch arms use this to leave a
    /// shared post-branch continuation for the enclosing block instead of whichever arm happens
    /// to be visited first.
    fn walk_from_until(&mut self, node_id: &str, stop_before: Option<&str>) -> Block {
        let mut block = Block::default();
        let mut current = Some(node_id.to_string());

        while let Some(nid) = current.take() {
            if stop_before == Some(nid.as_str()) {
                break;
            }
            if self.visited.contains(&nid) {
                break;
            }
            self.visited.insert(nid.clone());
            let Some(node) = self.nodes_by_id.get(nid.as_str()).copied() else {
                break;
            };

            let call = self.build_call(node);
            let exec_outs = exec_output_pins(node);

            // Loop nodes: open a body block from `exec_out` and continue the enclosing chain
            // from `done`, binding the loop handle (its `value`/`index`/`iter` outputs).
            if let Some(keyword) = loop_keyword(node) {
                let body = match self.exec_target_by_name(node, LOOP_BODY_PIN) {
                    Some(target) => self.walk_from(&target),
                    None => Block::default(),
                };
                block.stmts.push(Stmt::Loop {
                    keyword: keyword.to_string(),
                    bind: self.bindings.get(&node.id).cloned(),
                    call,
                    body,
                    anchor: Some(node.id.clone()),
                });
                current = self.exec_target_by_name(node, LOOP_DONE_PIN);
                continue;
            }

            // Conditional branch: render `if (cond) { } [else { }]`, with the connected exec
            // outputs as the arms. A common postdominating fan-in resumes the enclosing block.
            if node.name == CONTROL_BRANCH {
                let condition = call
                    .args
                    .iter()
                    .find(|a| a.name == CONDITION_PIN)
                    .map(|a| a.value.clone());
                let (arms, join) = self.lower_exec_arms(node, &exec_outs, stop_before);
                block.stmts.push(Stmt::Branch {
                    bind: None,
                    call,
                    condition,
                    arms,
                    anchor: Some(node.id.clone()),
                });
                current = join;
                continue;
            }

            if exec_outs.len() <= 1 {
                block.stmts.push(self.make_stmt(node, call));
                current = exec_outs.first().and_then(|p| self.first_exec_target(p));
            } else {
                // Multi-output execution cannot be represented as plain statement order without
                // choosing a branch. Render the actual connected outputs as labelled arms so the
                // reverse direction can preserve success/error/custom branches instead of
                // flattening them into a guessed linear path.
                let (arms, join) = self.lower_exec_arms(node, &exec_outs, stop_before);
                block.stmts.push(Stmt::Branch {
                    bind: self.bindings.get(&node.id).cloned(),
                    call,
                    condition: None,
                    arms,
                    anchor: Some(node.id.clone()),
                });
                current = join;
            }
        }

        block
    }

    /// Lower every connected execution output as a labelled arm. When all arms must pass through
    /// one nearest fan-in node, stop each arm immediately before that node and return it as the
    /// enclosing continuation. Nested arms also inherit their caller's stop so they cannot consume
    /// a join owned by an outer branch when they have no nearer structured join of their own.
    fn lower_exec_arms(
        &mut self,
        node: &'a Node,
        exec_outs: &[&'a Pin],
        enclosing_stop: Option<&str>,
    ) -> (Vec<BranchArm>, Option<String>) {
        let join = self.shared_exec_join(exec_outs);
        let arm_stop = join.as_deref().or(enclosing_stop);
        let mut arms = Vec::with_capacity(exec_outs.len());
        for pin in exec_outs {
            let body = match self.first_exec_target(pin) {
                Some(target) => self.walk_from_until(&target, arm_stop),
                None => Block::default(),
            };
            arms.push(BranchArm {
                label: arm_label(node, pin),
                body,
            });
        }
        (arms, join)
    }

    /// Find the nearest execution node that every connected output must reach. Reconcile models
    /// statements after a branch as a legal multi-source exec input; lowering must recognize that
    /// fan-in explicitly rather than relying on global `visited`, which nests the continuation in
    /// whichever arm is traversed first.
    fn shared_exec_join(&self, exec_outs: &[&Pin]) -> Option<String> {
        if exec_outs.len() < 2 {
            return None;
        }

        let starts: Vec<String> = exec_outs
            .iter()
            .map(|pin| self.first_exec_target(pin))
            .collect::<Option<_>>()?;
        let distances: Vec<HashMap<String, usize>> = starts
            .iter()
            .map(|start| self.exec_distances(start))
            .collect();
        let first = distances.first()?;

        first
            .keys()
            .filter(|candidate| {
                !self.visited.contains(candidate.as_str())
                    && self.is_exec_fan_in(candidate)
                    && distances
                        .iter()
                        .all(|reachable| reachable.contains_key(candidate.as_str()))
                    && starts
                        .iter()
                        .all(|start| self.exec_postdominates(start, candidate))
            })
            .min_by_key(|candidate| {
                let max_distance = distances
                    .iter()
                    .filter_map(|reachable| reachable.get(candidate.as_str()))
                    .copied()
                    .max()
                    .unwrap_or(usize::MAX);
                let total_distance = distances
                    .iter()
                    .filter_map(|reachable| reachable.get(candidate.as_str()))
                    .copied()
                    .sum::<usize>();
                (max_distance, total_distance, (*candidate).clone())
            })
            .cloned()
    }

    /// Breadth-first execution distance from one arm target. Distances make the first common
    /// postdominator win when several nodes in the shared continuation satisfy the predicate.
    fn exec_distances(&self, start: &str) -> HashMap<String, usize> {
        let mut distances = HashMap::from([(start.to_string(), 0usize)]);
        let mut queue = VecDeque::from([start.to_string()]);
        while let Some(current) = queue.pop_front() {
            let next_distance = distances[&current].saturating_add(1);
            let (successors, _) = self.exec_successors(&current);
            for successor in successors {
                if distances.contains_key(&successor) {
                    continue;
                }
                distances.insert(successor.clone(), next_distance);
                queue.push_back(successor);
            }
        }
        distances
    }

    /// Whether every execution path from `start` reaches `candidate`. A reachable cycle or
    /// terminal output that avoids the candidate fails closed, preventing an unsafe hoist from a
    /// merely common-but-optional downstream node.
    fn exec_postdominates(&self, start: &str, candidate: &str) -> bool {
        fn all_paths_reach(
            lowering: &Lowering<'_>,
            current: &str,
            candidate: &str,
            visiting: &mut HashSet<String>,
            memo: &mut HashMap<String, bool>,
        ) -> bool {
            if current == candidate {
                return true;
            }
            if let Some(result) = memo.get(current) {
                return *result;
            }
            if !visiting.insert(current.to_string()) {
                return false;
            }

            let (successors, has_terminal_output) = lowering.exec_successors(current);
            let result = !has_terminal_output
                && !successors.is_empty()
                && successors.iter().all(|successor| {
                    all_paths_reach(lowering, successor, candidate, visiting, memo)
                });
            visiting.remove(current);
            memo.insert(current.to_string(), result);
            result
        }

        all_paths_reach(
            self,
            start,
            candidate,
            &mut HashSet::new(),
            &mut HashMap::new(),
        )
    }

    /// Downstream execution nodes plus whether this node has an output that terminates without
    /// reaching another board node. The latter matters for postdominance: one unconnected branch
    /// must prevent a downstream node from being hoisted as unconditional.
    fn exec_successors(&self, node_id: &str) -> (Vec<String>, bool) {
        let Some(node) = self.nodes_by_id.get(node_id).copied() else {
            return (Vec::new(), true);
        };
        let mut outputs: Vec<&Pin> = node
            .pins
            .values()
            .filter(|pin| pin.pin_type == PinType::Output && is_exec(pin))
            .collect();
        outputs.sort_by_key(|pin| (pin.index, pin.id.as_str()));
        if outputs.is_empty() {
            return (Vec::new(), true);
        }

        let mut successors = Vec::new();
        let mut has_terminal_output = false;
        for output in outputs {
            match self.first_exec_target(output) {
                Some(target) => successors.push(target),
                None => has_terminal_output = true,
            }
        }
        successors.sort();
        successors.dedup();
        (successors, has_terminal_output)
    }

    fn is_exec_fan_in(&self, node_id: &str) -> bool {
        self.nodes_by_id.get(node_id).is_some_and(|node| {
            node.pins.values().any(|pin| {
                pin.pin_type == PinType::Input && is_exec(pin) && pin.depends_on.len() > 1
            })
        })
    }

    fn make_stmt(&mut self, node: &'a Node, call: Call) -> Stmt {
        // `structSet` chains render as a readable mutable accumulator while keeping the
        // underlying node call visible/anchored and therefore reversible.
        if node.name == STRUCT_SET
            && let Some(target) = self.struct_accumulators.get(&node.id)
            && let Some(output) = self.struct_set_output_pin(node)
        {
            // A single-field accumulator reassignment (`struct_in` reads the same variable the node
            // rebinds and `field` is a literal) is the readable `base.path = value` struct-field
            // write. Dynamic-field or cross-source updates keep the explicit `structSet({…})` form.
            if self.previous_struct_set(node).is_some()
                && let Some((path, value)) = struct_set_field_assign(&call, target)
            {
                return Stmt::FieldAssign {
                    base: target.clone(),
                    path,
                    value,
                    anchor: Some(node.id.clone()),
                };
            }

            let value = Expr::Field {
                base: Box::new(Expr::Call(call)),
                pin: output.name.clone(),
            };
            if self.previous_struct_set(node).is_none() {
                return Stmt::LocalAlias {
                    name: target.clone(),
                    value,
                    anchor: Some(node.id.clone()),
                };
            }
            return Stmt::Assign {
                target: target.clone(),
                value,
                anchor: Some(node.id.clone()),
            };
        }

        // `variable_set` sugars to a plain assignment `name = value`.
        if node.name == VARIABLE_SET
            && let Some(target) = self.var_name_of(node) {
                let value = self
                    .input_expr(node, "value_in")
                    .unwrap_or(Expr::Literal(Literal::Null));
                return Stmt::Assign {
                    target,
                    value,
                    anchor: Some(node.id.clone()),
                };
            }
        // `events_generic_return_result` sugars to a `return <response>` statement. Keep the node
        // id as the anchor so reconcile matches the existing result node instead of duplicating it.
        if node.name == EVENT_RETURN_RESULT {
            let value = self.input_expr(node, EVENT_RESPONSE_PIN);
            return Stmt::Return {
                values: value.into_iter().collect(),
                anchor: Some(node.id.clone()),
            };
        }
        if let Some(name) = self.bindings.get(&node.id) {
            Stmt::Let {
                name: name.clone(),
                call,
                anchor: Some(node.id.clone()),
            }
        } else {
            Stmt::Call {
                call,
                anchor: Some(node.id.clone()),
            }
        }
    }

    /// Build a call expression for a node, resolving its connected/literal data inputs.
    fn build_call(&mut self, node: &Node) -> Call {
        let mut data_inputs: Vec<&Pin> = node
            .pins
            .values()
            .filter(|p| p.pin_type == PinType::Input && !is_exec(p))
            .collect();
        data_inputs.sort_by_key(|p| p.index);

        let mut args = Vec::new();
        for pin in data_inputs {
            if let Some(source_pin_id) = pin.depends_on.iter().next()
                && let Some(expr) = self.resolve_source(source_pin_id) {
                    args.push(Arg {
                        name: pin.name.clone(),
                        value: expr,
                    });
                    continue;
                }
            // No connection: include a configured literal default if present.
            if let Some(bytes) = &pin.default_value {
                // Most JSON `null` defaults mean "unset" and intentionally stay absent from
                // FlowScript. `struct_set.value` is different: an explicit null is the required,
                // semantic value used to clear a field, so dropping it makes the rendered call
                // invalid and prevents a lossless round-trip.
                let lit = if node.name == STRUCT_SET && pin.name == STRUCT_SET_VALUE_PIN {
                    util::decode_default_preserving_null(bytes)
                } else {
                    util::decode_default(bytes)
                };
                if let Some(lit) = lit {
                    // `controlCallReference.fnRef` holds an opaque target node id; resolve it to
                    // that node's binding/display name instead of leaking the CUID.
                    if node.name == CALL_REFERENCE && pin.name == FN_REF_PIN
                        && let Some(name) = self.node_ref_name(&lit) {
                            args.push(Arg {
                                name: pin.name.clone(),
                                value: Expr::Ref(name),
                            });
                            continue;
                        }
                    args.push(Arg {
                        name: pin.name.clone(),
                        value: self.sugar_literal(lit),
                    });
                }
            }
        }

        let (display, mut args) = sugar_call(node, args);
        if let Some(tools) = self.fn_ref_arg(node) {
            args.push(tools);
        }
        Call {
            node_type: node.name.clone(),
            display,
            args,
            anchor: Some(node.id.clone()),
        }
    }

    /// Synthesize a `tools:`/`fnRefs:` argument from a node's `fn_refs`, surfacing the referenced
    /// tool/function entries as a bare reference array. Returns `None` when the node holds no
    /// resolvable references.
    fn fn_ref_arg(&self, node: &Node) -> Option<Arg> {
        let fn_refs = node.fn_refs.as_ref()?;
        if !fn_refs.can_reference_fns || fn_refs.fn_refs.is_empty() {
            return None;
        }
        let refs: Vec<Expr> = fn_refs
            .fn_refs
            .iter()
            .filter_map(|id| self.node_ref_name(&Literal::String(id.clone())))
            .map(Expr::Ref)
            .collect();
        if refs.is_empty() {
            return None;
        }
        let name = if AGENT_REGISTER_TOOLS.contains(&node.name.as_str()) {
            TOOLS_ARG
        } else {
            FN_REFS_ARG
        };
        Some(Arg {
            name: name.to_string(),
            value: Expr::Array(refs),
        })
    }

    /// Resolve the expression that produces the value on `output_pin_id`.
    fn resolve_source(&mut self, output_pin_id: &str) -> Option<Expr> {
        // Collapse reroute pass-through nodes: they are pure visual wire-bends (`route_in` ->
        // `route_out`, no exec pins) and carry no semantics, so resolve straight to their
        // upstream source. The reroute ids/coords still live in the board and are restored on
        // reconcile, so dropping them from the clean text stays (semantically) lossless.
        if let Some(upstream) = self.reroute_passthrough(output_pin_id) {
            return self.resolve_source(&upstream);
        }

        // A presentational layer-boundary bridge pin carries a value across a collapsed/macro
        // sub-layer frame: resolve straight to the producer feeding the boundary's outer side.
        if let Some(upstream) = self.boundary_passthrough(output_pin_id) {
            return self.resolve_source(&upstream);
        }

        // A function-layer boundary input pin resolves to the parameter reference.
        if let Some(param) = self.boundary_params.get(output_pin_id) {
            return Some(Expr::Ref(param.clone()));
        }

        // An event-entry data output pin resolves to the event's bare parameter reference (the
        // payload field is declared in the event signature, so the body reads it by name).
        if let Some(param) = self.event_params.get(output_pin_id) {
            return Some(Expr::Ref(param.clone()));
        }
        let source_pin = *self.pins.get(output_pin_id)?;
        let owner = *self.pin_owner.get(output_pin_id)?;

        if owner.name == STRUCT_SET
            && let Some(name) = self.struct_accumulators.get(&owner.id)
        {
            return Some(Expr::Ref(name.clone()));
        }

        // Sugar known data-plumbing nodes into idiomatic expressions (variable reads, struct
        // literals/access) instead of leaking the raw `variableGet`/`structBreak` calls and the
        // mangled `__break_struct_field__` pin names. Cycle-guarded like the pure-inline path.
        if let Some(expr) = self.sugar_source(owner, source_pin) {
            return Some(expr);
        }

        // Impure source -> reference its binding and select the output pin.
        if is_impure(owner) {
            if let Some(name) = self.bindings.get(&owner.id) {
                return Some(Expr::Field {
                    base: Box::new(Expr::Ref(name.clone())),
                    pin: source_pin.name.clone(),
                });
            }
            // Impure but unbound (output not pre-registered): fall back to a bare ref.
            return Some(Expr::Ref(binding_base_name(owner)));
        }

        // Pure source -> inline the call (guard against cycles).
        if self.inlining.contains(&owner.id) {
            return Some(Expr::Ref(binding_base_name(owner)));
        }
        self.inlining.insert(owner.id.clone());
        let call = self.build_call(owner);
        self.inlining.remove(&owner.id);

        let data_outputs: Vec<&Pin> = owner
            .pins
            .values()
            .filter(|p| p.pin_type == PinType::Output && !is_exec(p))
            .collect();
        if data_outputs.len() > 1 {
            Some(Expr::Field {
                base: Box::new(Expr::Call(call)),
                pin: source_pin.name.clone(),
            })
        } else {
            Some(Expr::Call(call))
        }
    }

    /// If `output_pin_id` is a reroute node's output, return the upstream output pin id feeding
    /// that reroute's `route_in` (one hop). Chains collapse because `resolve_source` recurses.
    /// Returns `None` for non-reroute owners or an unconnected reroute.
    fn reroute_passthrough(&self, output_pin_id: &str) -> Option<String> {
        let owner = *self.pin_owner.get(output_pin_id)?;
        if owner.name != REROUTE_NODE {
            return None;
        }
        // Reroute has a single data input (`route_in`); follow its first dependency.
        let route_in = owner
            .pins
            .values()
            .find(|p| p.pin_type == PinType::Input && !is_exec(p))?;
        route_in.depends_on.iter().next().cloned()
    }

    /// If `pin_id` is a presentational layer-boundary bridge pin, return the output pin id feeding
    /// its outer (producer) side so `resolve_source` can see through the sub-layer frame. Nested
    /// frames collapse because `resolve_source` recurses.
    fn boundary_passthrough(&self, pin_id: &str) -> Option<String> {
        let bp = self.boundary_pins.get(pin_id)?;
        bp.depends_on.iter().next().cloned()
    }

    /// Sugar a data-plumbing source node into an idiomatic expression. Returns `None` for nodes
    /// that should keep their literal call form. `source_pin` is the specific output being read.
    fn sugar_source(&mut self, owner: &'a Node, source_pin: &'a Pin) -> Option<Expr> {
        // Comparison/arithmetic/logic nodes -> `lhs <op> rhs` (read by pin index).
        if let Some(op) = binary_op(owner)
            && let Some(expr) = self.binary_expr(owner, op) {
                return Some(expr);
            }
        match owner.name.as_str() {
            // `variableGet` -> bare variable reference.
            VARIABLE_GET => self.var_name_of(owner).map(Expr::Ref),
            // Reading a `variableSet`'s pass-through output is just the variable's value.
            VARIABLE_SET => self.var_name_of(owner).map(Expr::Ref),
            // `structMake` -> empty struct literal `{}`.
            STRUCT_MAKE => Some(Expr::Object(Vec::new())),
            // `structMakeFromSchema` -> `{ field: value, … }` from its dynamic field pins.
            STRUCT_MAKE_SCHEMA => Some(self.struct_object(owner)),
            // `makeArray` -> empty array literal `[]`.
            MAKE_ARRAY => Some(Expr::Array(Vec::new())),
            // `arrayLength` -> `base.length` member access.
            ARRAY_LENGTH => {
                let base = self.input_expr(owner, "array")?;
                Some(Expr::Member {
                    base: Box::new(base),
                    field: "length".to_string(),
                })
            }
            // `arrayGet` -> `base[index]` index access (only the `element` output; `success`
            // keeps the call form).
            ARRAY_GET if source_pin.name == "element" => {
                if self.inlining.contains(&owner.id) {
                    return Some(Expr::Ref(binding_base_name(owner)));
                }
                self.inlining.insert(owner.id.clone());
                let base = self.input_expr(owner, "array_in");
                let index = self.input_expr(owner, "index");
                self.inlining.remove(&owner.id);
                Some(Expr::Index {
                    base: Box::new(base?),
                    index: Box::new(index.unwrap_or(Expr::Literal(Literal::Int(0)))),
                })
            }
            // `utilsTypesSelect` -> `condition ? a : b` ternary.
            TYPES_SELECT => {
                if self.inlining.contains(&owner.id) {
                    return Some(Expr::Ref(binding_base_name(owner)));
                }
                self.inlining.insert(owner.id.clone());
                let cond = self.input_expr(owner, "condition");
                let then = self.input_expr(owner, "a");
                let otherwise = self.input_expr(owner, "b");
                self.inlining.remove(&owner.id);
                Some(Expr::Ternary {
                    cond: Box::new(cond.unwrap_or(Expr::Literal(Literal::Bool(true)))),
                    then: Box::new(then.unwrap_or(Expr::Literal(Literal::Null))),
                    otherwise: Box::new(otherwise.unwrap_or(Expr::Literal(Literal::Null))),
                })
            }
            // `structGet` -> `base.field` member access (only when the field key is a literal).
            STRUCT_GET => {
                let base = self.input_expr(owner, "struct")?;
                let field = self.pin_literal_string(owner, "field")?;
                Some(Expr::Member {
                    base: Box::new(base),
                    field,
                })
            }
            // `structBreak` -> `base.field` member access; the field is encoded in the pin name.
            STRUCT_BREAK => {
                let field = source_pin
                    .name
                    .strip_prefix(BREAK_STRUCT_PREFIX)
                    .unwrap_or(&source_pin.name)
                    .to_string();
                let base = self.input_expr(owner, "struct_in")?;
                Some(Expr::Member {
                    base: Box::new(base),
                    field,
                })
            }
            _ => None,
        }
    }

    /// Build a `lhs <op> rhs` binary expression from a two-input operator node. Returns `None`
    /// when the node does not expose exactly two data inputs (so it falls back to a plain call).
    fn binary_expr(&mut self, node: &'a Node, op: &str) -> Option<Expr> {
        if self.inlining.contains(&node.id) {
            return Some(Expr::Ref(binding_base_name(node)));
        }
        let mut inputs: Vec<&Pin> = node
            .pins
            .values()
            .filter(|p| p.pin_type == PinType::Input && !is_exec(p))
            .collect();
        inputs.sort_by_key(|p| p.index);
        if inputs.len() != 2 {
            return None;
        }
        self.inlining.insert(node.id.clone());
        let lhs = self
            .pin_expr(inputs[0])
            .unwrap_or(Expr::Literal(Literal::Null));
        let rhs = self
            .pin_expr(inputs[1])
            .unwrap_or(Expr::Literal(Literal::Null));
        self.inlining.remove(&node.id);
        Some(Expr::Binary {
            op: op.to_string(),
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
        })
    }

    /// Build a struct literal expression from a `structMakeFromSchema` node's dynamic field pins.
    fn struct_object(&mut self, node: &'a Node) -> Expr {
        if self.inlining.contains(&node.id) {
            return Expr::Ref(binding_base_name(node));
        }
        self.inlining.insert(node.id.clone());
        let mut field_pins: Vec<&Pin> = node
            .pins
            .values()
            .filter(|p| {
                p.pin_type == PinType::Input
                    && !is_exec(p)
                    && p.name.starts_with(MAKE_STRUCT_PREFIX)
            })
            .collect();
        field_pins.sort_by_key(|p| p.index);

        let mut fields = Vec::new();
        for pin in field_pins {
            let key = pin
                .name
                .strip_prefix(MAKE_STRUCT_PREFIX)
                .unwrap_or(&pin.name)
                .to_string();
            let value = self.pin_expr(pin).unwrap_or(Expr::Literal(Literal::Null));
            fields.push(ObjectField { key, value });
        }
        self.inlining.remove(&node.id);
        Expr::Object(fields)
    }

    /// Resolve the expression feeding a node's named input pin (connection or literal default).
    fn input_expr(&mut self, node: &'a Node, pin_name: &str) -> Option<Expr> {
        let pin = node
            .pins
            .values()
            .find(|p| p.name == pin_name && p.pin_type == PinType::Input)?;
        self.pin_expr(pin)
    }

    /// Resolve the expression feeding a specific input pin (connection first, else literal).
    fn pin_expr(&mut self, pin: &'a Pin) -> Option<Expr> {
        if let Some(source_pin_id) = pin.depends_on.iter().next()
            && let Some(expr) = self.resolve_source(source_pin_id) {
                return Some(expr);
            }
        let bytes = pin.default_value.as_ref()?;
        util::decode_default(bytes).map(|lit| self.sugar_literal(lit))
    }

    /// Read a String literal default off a node's named input pin.
    fn pin_literal_string(&self, node: &'a Node, pin_name: &str) -> Option<String> {
        let pin = node
            .pins
            .values()
            .find(|p| p.name == pin_name && p.pin_type == PinType::Input)?;
        let bytes = pin.default_value.as_ref()?;
        match util::decode_default(bytes)? {
            Literal::String(s) => Some(s),
            _ => None,
        }
    }

    /// The camelCase variable name backing a `variableGet`/`variableSet` node's `var_ref` pin.
    fn var_name_of(&self, node: &'a Node) -> Option<String> {
        let id = self.pin_literal_string(node, "var_ref")?;
        self.var_names.get(id.as_str()).cloned()
    }

    /// Resolve a `controlCallReference` target node id literal to that node's display name (its
    /// binding name if it has one, otherwise its friendly/type base name).
    fn node_ref_name(&self, lit: &Literal) -> Option<String> {
        let Literal::String(id) = lit else {
            return None;
        };
        let node = self.nodes_by_id.get(id.as_str())?;
        Some(
            self.bindings
                .get(&node.id)
                .cloned()
                .unwrap_or_else(|| binding_base_name(node)),
        )
    }

    /// Replace an opaque function-layer id literal with a bare function reference; leave all
    /// other literals untouched. CUIDs are unique, so a false match is effectively impossible.
    fn sugar_literal(&self, lit: Literal) -> Expr {
        if let Literal::String(ref s) = lit
            && let Some(name) = self.fn_names.get(s.as_str()) {
                return Expr::Ref(name.clone());
            }
        Expr::Literal(lit)
    }

    /// First downstream node connected to an exec output pin.
    fn first_exec_target(&self, pin: &Pin) -> Option<String> {
        let mut targets: Vec<String> = self
            .resolve_targets(&pin.connected_to)
            .into_iter()
            .map(|n| n.id.clone())
            .collect();
        targets.sort();
        targets.into_iter().next()
    }

    /// Expand connection target pin ids into their owning nodes, following presentational
    /// layer-boundary bridge pins (forward, via `connected_to`) transparently so an edge that
    /// crosses a collapsed/macro sub-layer resolves to the real downstream node.
    fn resolve_targets<'b, I>(&self, targets: I) -> Vec<&'a Node>
    where
        I: IntoIterator<Item = &'b String>,
    {
        let mut out = Vec::new();
        let mut stack: Vec<&str> = targets.into_iter().map(|s| s.as_str()).collect();
        let mut seen: HashSet<&str> = HashSet::new();
        while let Some(pid) = stack.pop() {
            if !seen.insert(pid) {
                continue;
            }
            if let Some(node) = self.pin_owner.get(pid) {
                out.push(*node);
            } else if let Some(bp) = self.boundary_pins.get(pid) {
                stack.extend(bp.connected_to.iter().map(|s| s.as_str()));
            }
        }
        out
    }

    /// Expand connection source pin ids into their owning nodes, following presentational
    /// layer-boundary bridge pins (backward, via `depends_on`) transparently.
    fn resolve_sources<'b, I>(&self, sources: I) -> Vec<&'a Node>
    where
        I: IntoIterator<Item = &'b String>,
    {
        let mut out = Vec::new();
        let mut stack: Vec<&str> = sources.into_iter().map(|s| s.as_str()).collect();
        let mut seen: HashSet<&str> = HashSet::new();
        while let Some(pid) = stack.pop() {
            if !seen.insert(pid) {
                continue;
            }
            if let Some(node) = self.pin_owner.get(pid) {
                out.push(*node);
            } else if let Some(bp) = self.boundary_pins.get(pid) {
                stack.extend(bp.depends_on.iter().map(|s| s.as_str()));
            }
        }
        out
    }

    /// First downstream node connected to the named exec output pin of `node`.
    fn exec_target_by_name(&self, node: &Node, pin_name: &str) -> Option<String> {
        let pin = node
            .pins
            .values()
            .find(|p| p.pin_type == PinType::Output && is_exec(p) && p.name == pin_name)?;
        self.first_exec_target(pin)
    }

    /// A node is an exec entry within `scope` if it is a declared `start` node, or it has an exec
    /// output and none of its exec inputs are fed by another node in the same scope (i.e. it is
    /// fed only by the scope's boundary, an external scope, or nothing). An `event_callback` node
    /// that is fed mid-chain is therefore part of that chain, not an independent entry.
    fn is_scope_entry(&self, node: &Node, scope_ids: &HashSet<&str>) -> bool {
        if node.start == Some(true) {
            return true;
        }
        let has_exec_output = node
            .pins
            .values()
            .any(|p| p.pin_type == PinType::Output && is_exec(p));
        if !has_exec_output {
            return false;
        }
        let exec_inputs = node
            .pins
            .values()
            .filter(|p| p.pin_type == PinType::Input && is_exec(p));
        for pin in exec_inputs {
            for source_node in self.resolve_sources(&pin.depends_on) {
                if scope_ids.contains(source_node.id.as_str()) {
                    return false;
                }
            }
        }
        true
    }
}

fn interfaces_for_board_text_surfaces(board: &Board) -> Vec<InterfaceDecl> {
    let mut schema_sources = lower_variables(board.variables.values(), &board.refs);

    let mut layers = board.layers.values().collect::<Vec<_>>();
    layers.sort_by(|left, right| left.id.cmp(&right.id));
    for layer in &layers {
        let mut variables = layer.variables.values().collect::<Vec<_>>();
        variables.sort_by(|left, right| left.name.cmp(&right.name).then(left.id.cmp(&right.id)));
        schema_sources.extend(
            variables
                .into_iter()
                .map(|variable| var_decl_of(variable, &board.refs)),
        );

        if matches!(layer.r#type, LayerType::Function) {
            let mut pins = layer.pins.values().collect::<Vec<_>>();
            pins.sort_by_key(|pin| (pin.index, pin.id.clone()));
            for pin in pins {
                if let Some(source) =
                    schema_source_for_pin(&format!("{}_{}", layer.name, pin.name), pin, &board.refs)
                {
                    schema_sources.push(source);
                }
            }
        }
    }

    // Trigger payload outputs are the other pin contracts rendered as Params. Restrict this to
    // actual start/handler nodes: including every catalog Struct output would emit unused
    // interfaces, inflate the model prompt, and could rename an unrelated boundary interface.
    for indexed in canonical_board_nodes(board) {
        if indexed.node.start != Some(true) {
            continue;
        }
        let mut pins = indexed.node.pins.values().collect::<Vec<_>>();
        pins.sort_by_key(|pin| (pin.index, pin.id.clone()));
        for pin in pins {
            if pin.pin_type != PinType::Output || is_exec(pin) {
                continue;
            }
            if let Some(source) = schema_source_for_pin(
                &format!("{}_{}", indexed.node.name, pin.name),
                pin,
                &board.refs,
            ) {
                schema_sources.push(source);
            }
        }
    }

    flow_like_ast::interfaces_for_variables(&schema_sources)
}

fn schema_source_for_pin(name: &str, pin: &Pin, refs: &HashMap<String, String>) -> Option<VarDecl> {
    if pin
        .options
        .as_ref()
        .and_then(|options| options.enforce_schema)
        != Some(true)
    {
        return None;
    }
    let raw_schema = pin.schema.as_deref()?;
    let schema = refs
        .get(raw_schema)
        .cloned()
        .unwrap_or_else(|| raw_schema.to_string());
    Some(VarDecl {
        name: util::to_camel_case(name),
        ty: util::type_ref(&pin.data_type, &pin.value_type),
        default: None,
        exposed: false,
        secret: false,
        editable: true,
        runtime_configured: false,
        category: None,
        description: None,
        schema: Some(schema),
        anchor: None,
    })
}

fn lower_variables<'a, I>(variables: I, refs: &HashMap<String, String>) -> Vec<VarDecl>
where
    I: Iterator<Item = &'a Variable>,
{
    let mut vars: Vec<&Variable> = variables.collect();
    vars.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
    vars.iter().map(|v| var_decl_of(v, refs)).collect()
}

fn is_exec(pin: &Pin) -> bool {
    pin.data_type == VariableType::Execution
}

/// Build a `VarDecl` from a board/layer variable. `refs` is the board's schema/description ref
/// table (`hash -> full string`); `schema` and `description` are stored as ref hashes, so they
/// are resolved back to their literal content for the clean text (falling back to the raw value
/// when it is already inline / not a known ref).
fn var_decl_of(v: &Variable, refs: &HashMap<String, String>) -> VarDecl {
    let resolve = |value: &str| {
        refs.get(value)
            .cloned()
            .unwrap_or_else(|| value.to_string())
    };
    VarDecl {
        name: util::to_camel_case(&v.name),
        ty: util::type_ref(&v.data_type, &v.value_type),
        // Secret values must never enter the text domain: rendered FlowScript is shown in
        // editors, copied, and sent to LLMs. Reconcile lowers the live board through this
        // same path, so both sides agree the decl is value-free and round-trips can neither
        // leak nor clear the stored value.
        default: if v.secret {
            None
        } else {
            v.default_value
                .as_ref()
                .and_then(|b| util::decode_default(b))
        },
        exposed: v.exposed,
        secret: v.secret,
        editable: v.editable,
        runtime_configured: v.runtime_configured,
        category: v.category.clone(),
        description: v.description.as_deref().map(resolve),
        schema: v.schema.as_deref().map(resolve),
        anchor: Some(v.id.clone()),
    }
}

/// Resolve the display name (and pruned args) for a node call. `control_call_function` /
/// `control_call_reference` render as a direct call to the referenced function/node, dropping
/// the opaque id argument; all other nodes keep their camelCase type name and arguments.
fn sugar_call(node: &Node, mut args: Vec<Arg>) -> (String, Vec<Arg>) {
    match node.name.as_str() {
        CALL_FUNCTION => {
            if let Some(name) = ref_name_of_arg(&args, FUNCTION_LAYER_ID_PIN) {
                args.retain(|a| a.name != FUNCTION_LAYER_ID_PIN);
                return (name, args);
            }
        }
        CALL_REFERENCE => {
            if let Some(name) = ref_name_of_arg(&args, FN_REF_PIN) {
                args.retain(|a| a.name != FN_REF_PIN);
                return (name, args);
            }
        }
        _ => {}
    }
    (util::to_camel_case(&node.name), args)
}

/// If the named argument holds a bare reference (a resolved function/node name), return it.
fn ref_name_of_arg(args: &[Arg], pin: &str) -> Option<String> {
    args.iter()
        .find(|a| a.name == pin)
        .and_then(|a| match &a.value {
            Expr::Ref(name) => Some(name.clone()),
            _ => None,
        })
}

/// Collect every node call in one lexical block, including calls nested in argument expressions.
/// Nested handlers intentionally are not traversed: they are independent scopes and own their
/// registrations themselves.
fn collect_calls_in_block<'a>(block: &'a Block, calls: &mut Vec<&'a Call>) {
    for statement in &block.stmts {
        match statement {
            Stmt::Let { call, .. } | Stmt::Call { call, .. } => {
                collect_calls_in_call(call, calls);
            }
            Stmt::Branch {
                call,
                condition,
                arms,
                ..
            } => {
                collect_calls_in_call(call, calls);
                if let Some(condition) = condition {
                    collect_calls_in_expr(condition, calls);
                }
                for arm in arms {
                    collect_calls_in_block(&arm.body, calls);
                }
            }
            Stmt::Loop { call, body, .. } => {
                collect_calls_in_call(call, calls);
                collect_calls_in_block(body, calls);
            }
            Stmt::Assign { value, .. }
            | Stmt::FieldAssign { value, .. }
            | Stmt::LocalAlias { value, .. } => collect_calls_in_expr(value, calls),
            Stmt::Return { values, .. } => {
                for value in values {
                    collect_calls_in_expr(value, calls);
                }
            }
            Stmt::Handler(_) | Stmt::Local(_) | Stmt::Comment(_) => {}
        }
    }
}

fn collect_calls_in_call<'a>(call: &'a Call, calls: &mut Vec<&'a Call>) {
    calls.push(call);
    for arg in &call.args {
        collect_calls_in_expr(&arg.value, calls);
    }
}

fn collect_calls_in_expr<'a>(expr: &'a Expr, calls: &mut Vec<&'a Call>) {
    match expr {
        Expr::Call(call) => collect_calls_in_call(call, calls),
        Expr::Field { base, .. } | Expr::Member { base, .. } => collect_calls_in_expr(base, calls),
        Expr::Object(fields) => {
            for field in fields {
                collect_calls_in_expr(&field.value, calls);
            }
        }
        Expr::Array(values) => {
            for value in values {
                collect_calls_in_expr(value, calls);
            }
        }
        Expr::Index { base, index } => {
            collect_calls_in_expr(base, calls);
            collect_calls_in_expr(index, calls);
        }
        Expr::Ternary {
            cond,
            then,
            otherwise,
        } => {
            collect_calls_in_expr(cond, calls);
            collect_calls_in_expr(then, calls);
            collect_calls_in_expr(otherwise, calls);
        }
        Expr::Binary { lhs, rhs, .. } => {
            collect_calls_in_expr(lhs, calls);
            collect_calls_in_expr(rhs, calls);
        }
        Expr::Ref(_) | Expr::Literal(_) => {}
    }
}

fn is_impure(node: &Node) -> bool {
    node.pins.values().any(is_exec)
}

/// If a `struct_set` call is a single-field accumulator update of `target` — its `struct_in`
/// reads the same variable the node rebinds and its `field` is a literal string — return the
/// `(field_path, value_expr)` backing the `target.field = value` struct-field write sugar.
/// Returns `None` (keep the explicit `structSet({…})` form) when the field is wired/dynamic or
/// `struct_in` comes from a different source than `target`.
fn struct_set_field_assign(call: &Call, target: &str) -> Option<(String, Expr)> {
    let mut struct_in = None;
    let mut field = None;
    let mut value = None;
    for arg in &call.args {
        match arg.name.as_str() {
            STRUCT_SET_IN_PIN => struct_in = Some(&arg.value),
            STRUCT_SET_FIELD_PIN => field = Some(&arg.value),
            STRUCT_SET_VALUE_PIN => value = Some(&arg.value),
            _ => {}
        }
    }
    match struct_in {
        Some(Expr::Ref(name)) if name == target => {}
        _ => return None,
    }
    let Some(Expr::Literal(Literal::String(path))) = field else {
        return None;
    };
    Some((path.clone(), value?.clone()))
}

/// Variables materialized by reconcile for a literal `return` are named `{fn}_{pin}`
/// (historically with a `_N` uniqueness suffix from the pre-reuse planner).
fn is_materialized_return_name(variable_name: &str, fn_name: &str, pin_name: &str) -> bool {
    let base = format!("{fn_name}_{pin_name}");
    variable_name == base
        || variable_name
            .strip_prefix(&base)
            .and_then(|rest| rest.strip_prefix('_'))
            .is_some_and(|suffix| {
                !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
            })
}

/// A trigger entry is a `start` node — an independent entry point (e.g. a generic event used as
/// an agent tool) whose data outputs are its payload. Unlike `event_callback` nodes (which run
/// inline as steps of the surrounding flow), a trigger never has an incoming exec edge, so inside
/// a function scope it is rendered as its own nested handler rather than part of the linear chain.
fn is_trigger_entry(node: &Node) -> bool {
    node.start == Some(true)
}

fn exec_output_pins(node: &Node) -> Vec<&Pin> {
    let mut pins: Vec<&Pin> = node
        .pins
        .values()
        .filter(|p| p.pin_type == PinType::Output && is_exec(p) && !p.connected_to.is_empty())
        .collect();
    pins.sort_by_key(|p| p.index);
    pins
}

/// Branch arm label for one exec output pin. Same-named siblings (repeatable exec pins such as
/// `control_par_execution`'s `exec_out`) get the stable positional selector board commands use
/// (`name[#N]`, occurrence among ALL same-named exec outputs sorted by index/id) so each arm
/// addresses exactly one pin on the reverse path; the first occurrence keeps the plain name.
fn arm_label(node: &Node, pin: &Pin) -> String {
    let mut same_named: Vec<&Pin> = node
        .pins
        .values()
        .filter(|p| p.pin_type == PinType::Output && is_exec(p) && p.name == pin.name)
        .collect();
    if same_named.len() <= 1 {
        return pin.name.clone();
    }
    same_named.sort_by_key(|p| (p.index, p.id.clone()));
    match same_named.iter().position(|p| p.id == pin.id) {
        Some(0) | None => pin.name.clone(),
        Some(occurrence) => super::pin_occurrence_ref(&pin.name, occurrence),
    }
}

fn binding_base_name(node: &Node) -> String {
    let source = if !node.friendly_name.trim().is_empty() {
        &node.friendly_name
    } else {
        &node.name
    };
    util::to_camel_case(source)
}

fn unique_name(base: &str, used: &mut HashSet<String>) -> String {
    if used.insert(base.to_string()) {
        return base.to_string();
    }
    let mut n = 2;
    loop {
        let candidate = format!("{base}{n}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
        n += 1;
    }
}

/// Event blocks are ordered by node coordinates (top-left first) for stable output.
fn entry_order(node: &Node) -> (i64, i64) {
    node.coordinates
        .map(|(x, y, _)| (y as i64, x as i64))
        .unwrap_or((i64::MAX, i64::MAX))
}

/// Canonical FlowScript selector for an event entry's exact catalog type. Keeping the type in the
/// first header slot means reparsing rendered FlowScript can still recreate the same entry when
/// its identity anchor has gone stale.
fn event_type_name(node: &Node) -> String {
    util::to_camel_case(&node.name)
}

/// Optional human-facing name for one event entry. The renderer places this after the catalog
/// selector (`eventsSimple dashboardLoad()`), keeping identity and presentation independent.
fn event_alias(node: &Node) -> Option<String> {
    let friendly_name = node.friendly_name.trim();
    if friendly_name.is_empty() {
        return None;
    }

    let alias = util::to_camel_case(friendly_name);
    (alias != event_type_name(node)).then_some(alias)
}
