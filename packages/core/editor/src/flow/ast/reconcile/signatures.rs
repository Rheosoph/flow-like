//! A function whose written signature differs from its layer is migrated instead of refused.
//!
//! The layer's data boundary becomes the written parameters and returns
//! (`BoardCommand::UpdateFunctionSignature`), and every call node mirrors those pins once the
//! apply runs its `on_update`. The rest of the document is planned against a board where both have
//! already happened, so the body and every call site are wired by pin name to pins that only exist
//! after the migration.
//!
//! A pin whose name and direction survive keeps its id, and with it its wiring. A renamed
//! parameter is a removed one plus a new one that the text wires again. That is only safe when the
//! document writes every call of the function, so a migration that drops or retypes a pin is
//! refused while a call the document does not write would keep using the function.

use std::collections::{HashMap, HashSet};

use flow_like_ast::model::FnDecl;
use flow_like_types::create_id;

use crate::flow::{
    board::{Board, Layer, LayerType},
    copilot::{BoardCommand, PinMetadata, PlaceholderPinDef},
    node::Node,
    pin::{Pin, PinType},
};

use super::{
    CALL_FUNCTION_NODE_TYPE, FUNCTION_LAYER_ID_PIN, FlatDocument, ModuleResolution,
    boundary_pin_metadata, function_boundary_contract_matches, interface_schema_map,
    matching_function_layer, node_pin_literal_string, param_carries_pin_options,
    param_pin_metadata,
};

struct SignatureMigration {
    layer_id: String,
    function: String,
    boundary: Vec<PlaceholderPinDef>,
}

#[derive(Default)]
pub(super) struct SignatureMigrations {
    migrations: Vec<SignatureMigration>,
    pub(super) diagnostics: Vec<String>,
}

impl SignatureMigrations {
    /// Plan a migration for every written function whose parameters or returns no longer match
    /// its layer. `written_calls` holds the node ids the document anchors; `unwritten_calls_stay`
    /// says whether a call the document leaves out stays on the board (rather than being deleted).
    pub(super) fn plan(
        existing: &Board,
        doc: &FlatDocument,
        modules: &ModuleResolution,
        written_calls: &HashSet<String>,
        unwritten_calls_stay: bool,
    ) -> Self {
        let interface_schemas = interface_schema_map(&doc.ast);
        let claimed_layers = doc
            .ast
            .functions
            .iter()
            .filter_map(|func| func.anchor.as_ref())
            .filter(|anchor| {
                existing
                    .layers
                    .get(*anchor)
                    .is_some_and(|layer| matches!(layer.r#type, LayerType::Function))
            })
            .cloned()
            .collect::<HashSet<_>>();
        let mut planned = Self::default();
        for (index, func) in doc.ast.functions.iter().enumerate() {
            if func
                .params
                .iter()
                .chain(&func.returns)
                .any(param_carries_pin_options)
            {
                continue;
            }
            // Anchored functions retain their identity even when moved into a new module.
            // An unanchored declaration can only reuse a layer already in its written module.
            let module = if func.anchor.is_some() {
                None
            } else {
                let Some(module) = modules.board_module(&doc.function_scopes[index]) else {
                    continue;
                };
                module
            };
            let Some(layer) =
                matching_function_layer(existing, func, module.as_deref(), &claimed_layers)
            else {
                continue;
            };
            let Some(change) = signature_change(existing, layer, func, &interface_schemas) else {
                continue;
            };
            if !change.breaking.is_empty() && unwritten_calls_stay {
                let unwritten = function_calls(existing, &layer.id)
                    .filter(|node| !written_calls.contains(&node.id))
                    .map(|node| format!("`{}`", node.id))
                    .collect::<Vec<_>>();
                if !unwritten.is_empty() {
                    planned.diagnostics.push(format!(
                        "function `{}` changes {} in its signature, but {} call(s) of it are not part of this document ({}). Write every call of `{}` so it can be rewired, or keep the old signature",
                        func.name,
                        change.breaking.join(", "),
                        unwritten.len(),
                        unwritten.join(", "),
                        func.name
                    ));
                    continue;
                }
            }
            planned.migrations.push(SignatureMigration {
                layer_id: layer.id.clone(),
                function: func.name.clone(),
                boundary: change.boundary,
            });
        }
        planned
    }

    pub(super) fn commands(&self) -> Vec<BoardCommand> {
        self.migrations
            .iter()
            .map(|migration| BoardCommand::UpdateFunctionSignature {
                layer_id: migration.layer_id.clone(),
                pins: migration.boundary.clone(),
                summary: Some(format!(
                    "Change the signature of function {}",
                    migration.function
                )),
            })
            .collect()
    }

    /// The board with every migration applied: new layer boundaries and call nodes that mirror
    /// them, the edges of dropped pins cut. `None` when nothing migrates.
    pub(super) fn planning_board(&self, existing: &Board) -> Option<Board> {
        if self.migrations.is_empty() {
            return None;
        }
        let mut board = existing.clone();
        for migration in &self.migrations {
            let Some(layer) = board.layers.get_mut(&migration.layer_id) else {
                continue;
            };
            let Ok(pins) =
                super::super::apply::migrated_boundary_pins(&layer.pins, &migration.boundary)
            else {
                continue;
            };
            let mut dropped: HashSet<String> = layer
                .pins
                .keys()
                .filter(|id| !pins.contains_key(*id))
                .cloned()
                .collect();
            layer.pins = pins;
            let layer = layer.clone();
            for node in board.nodes.values_mut().chain(
                board
                    .layers
                    .values_mut()
                    .flat_map(|layer| layer.nodes.values_mut()),
            ) {
                if node.name == CALL_FUNCTION_NODE_TYPE
                    && node_pin_literal_string(node, FUNCTION_LAYER_ID_PIN).as_deref()
                        == Some(layer.id.as_str())
                {
                    dropped.extend(mirror_function_pins(node, &layer));
                }
            }
            cut_pins(&mut board, &dropped);
        }
        Some(board)
    }
}

struct SignatureChange {
    boundary: Vec<PlaceholderPinDef>,
    /// Parameters and returns that disappear or change type, as written in a diagnostic.
    breaking: Vec<String>,
}

fn names_match(live: &str, written: &str) -> bool {
    live == written
        || flow_like_ast::declared_identifier(live) == written
        || flow_like_ast::to_camel_case(live) == flow_like_ast::to_camel_case(written)
}

fn signature_change(
    existing: &Board,
    layer: &Layer,
    func: &FnDecl,
    interface_schemas: &HashMap<String, String>,
) -> Option<SignatureChange> {
    let live_params = crate::flow::ast::lower::function_params(layer);
    let live_returns: Vec<(&Pin, String)> = crate::flow::ast::lower::function_returns(layer);
    let written_params: Vec<PinMetadata> = func
        .params
        .iter()
        .map(|param| param_pin_metadata(param, interface_schemas))
        .collect();
    let written_returns: Vec<PinMetadata> = func
        .returns
        .iter()
        .map(|param| param_pin_metadata(param, interface_schemas))
        .collect();

    let matches = |live: &Pin, written: &PinMetadata| {
        function_boundary_contract_matches(&boundary_pin_metadata(live), written, &existing.refs)
            || erases_only_a_schema(live, written)
    };
    let params_match = live_params.len() == written_params.len()
        && live_params
            .iter()
            .zip(&written_params)
            .all(|(live, written)| matches(live, written));
    let returns_match = live_returns.len() == written_returns.len()
        && live_returns
            .iter()
            .zip(&written_returns)
            .all(|((pin, rendered), written)| {
                let mut live = boundary_pin_metadata(pin);
                if *rendered == written.name {
                    live.name = rendered.clone();
                }
                function_boundary_contract_matches(&live, written, &existing.refs)
                    || erases_only_a_schema(pin, written)
            });
    if params_match && returns_match {
        return None;
    }

    let mut kept: HashSet<&str> = HashSet::new();
    let mut breaking = Vec::new();
    let mut boundary = Vec::new();
    let sides = [
        (
            "Input",
            "parameter",
            written_params.as_slice(),
            live_params
                .iter()
                .map(|pin| (*pin, None))
                .collect::<Vec<_>>(),
        ),
        (
            "Output",
            "return",
            written_returns.as_slice(),
            live_returns
                .iter()
                .map(|(pin, rendered)| (*pin, Some(rendered.as_str())))
                .collect::<Vec<_>>(),
        ),
    ];
    for (pin_type, label, written, live) in sides {
        for written in written {
            let survivor = live.iter().find(|(pin, rendered)| {
                !kept.contains(pin.id.as_str())
                    && (rendered.is_some_and(|rendered| rendered == written.name)
                        || names_match(&pin.name, &written.name))
            });
            let def = match survivor {
                Some((pin, _)) => {
                    kept.insert(pin.id.as_str());
                    if matches(pin, written) {
                        placeholder(&boundary_pin_metadata(pin), &pin.name, pin_type)
                    } else {
                        breaking.push(format!("the type of {label} `{}`", written.name));
                        placeholder(written, &pin.name, pin_type)
                    }
                }
                None => placeholder(written, &written.name, pin_type),
            };
            boundary.push(def);
        }
        for (pin, rendered) in &live {
            if !kept.contains(pin.id.as_str()) {
                let name = rendered
                    .map(str::to_string)
                    .unwrap_or_else(|| flow_like_ast::declared_identifier(&pin.name));
                breaking.push(format!("{label} `{name}` (removed or renamed)"));
            }
        }
    }
    Some(SignatureChange { boundary, breaking })
}

/// Text that predates interfaces spells a schema-bearing boundary as a plain `Struct`. That is no
/// request to drop the schema, so it never migrates; the planner reports the mismatch instead.
fn erases_only_a_schema(live: &Pin, written: &PinMetadata) -> bool {
    let live = boundary_pin_metadata(live);
    live.data_type == "Struct"
        && written.data_type == "Struct"
        && live.value_type == written.value_type
        && live.schema.is_some()
        && written.schema.is_none()
        && names_match(&live.name, &written.name)
}

fn placeholder(meta: &PinMetadata, name: &str, pin_type: &str) -> PlaceholderPinDef {
    PlaceholderPinDef {
        name: name.to_string(),
        friendly_name: name.to_string(),
        description: None,
        pin_type: pin_type.to_string(),
        data_type: meta.data_type.clone(),
        value_type: Some(meta.value_type.clone()),
        schema: meta.schema.clone(),
        enforce_schema: meta.enforce_schema,
        optional: false,
        default_value: None,
    }
}

fn function_calls<'a>(existing: &'a Board, layer_id: &'a str) -> impl Iterator<Item = &'a Node> {
    existing
        .nodes
        .values()
        .chain(
            existing
                .layers
                .values()
                .flat_map(|layer| layer.nodes.values()),
        )
        .filter(move |node| {
            node.name == CALL_FUNCTION_NODE_TYPE
                && node_pin_literal_string(node, FUNCTION_LAYER_ID_PIN).as_deref() == Some(layer_id)
        })
}

/// What a call node's `on_update` does when its layer's pins change: mirror every layer pin by
/// name and direction, drop the rest. Returns the ids of the pins it dropped.
fn mirror_function_pins(node: &mut Node, layer: &Layer) -> Vec<String> {
    let mut wanted: HashSet<(String, bool)> = HashSet::new();
    wanted.insert((FUNCTION_LAYER_ID_PIN.to_string(), true));
    for pin_type in [PinType::Input, PinType::Output] {
        let mut layer_pins: Vec<&Pin> = layer
            .pins
            .values()
            .filter(|pin| pin.pin_type == pin_type)
            .collect();
        layer_pins.sort_by_key(|pin| pin.index);
        for layer_pin in layer_pins {
            wanted.insert((layer_pin.name.clone(), pin_type == PinType::Input));
            let mirror = node
                .pins
                .values_mut()
                .find(|pin| pin.name == layer_pin.name && pin.pin_type == pin_type);
            match mirror {
                Some(mirror) => {
                    mirror.data_type = layer_pin.data_type.clone();
                    mirror.value_type = layer_pin.value_type.clone();
                    mirror.schema = layer_pin.schema.clone();
                    mirror.options = layer_pin.options.clone();
                }
                None => {
                    let id = create_id();
                    let mut pin = layer_pin.clone();
                    pin.id = id.clone();
                    pin.connected_to.clear();
                    pin.depends_on.clear();
                    pin.default_value = None;
                    node.pins.insert(id, pin);
                }
            }
        }
    }
    let dropped: Vec<String> = node
        .pins
        .values()
        .filter(|pin| !wanted.contains(&(pin.name.clone(), pin.pin_type == PinType::Input)))
        .map(|pin| pin.id.clone())
        .collect();
    node.pins.retain(|id, _| !dropped.contains(id));
    dropped
}

/// Remove every reference to `pins` from the pins that remain.
fn cut_pins(board: &mut Board, pins: &HashSet<String>) {
    if pins.is_empty() {
        return;
    }
    let detach = |pin: &mut Pin| {
        pin.connected_to.retain(|id| !pins.contains(id));
        pin.depends_on.retain(|id| !pins.contains(id));
    };
    for node in board.nodes.values_mut().chain(
        board
            .layers
            .values_mut()
            .flat_map(|layer| layer.nodes.values_mut()),
    ) {
        node.pins.values_mut().for_each(detach);
    }
    for layer in board.layers.values_mut() {
        layer.pins.values_mut().for_each(detach);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::variable::VariableType;

    fn board() -> Board {
        Board::new_detached(None, flow_like_storage::Path::default())
    }

    fn add_module(board: &mut Board, id: &str, name: &str) {
        let layer = Layer::new(id.to_string(), name.to_string(), LayerType::Module);
        board.layers.insert(layer.id.clone(), layer);
    }

    fn add_function(board: &mut Board, id: &str, name: &str, module: Option<&str>) {
        let mut layer = Layer::new(id.to_string(), name.to_string(), LayerType::Function);
        layer.parent_id = module.map(str::to_string);
        let mut boundary = Node::new("boundary", "Boundary", "", "test");
        let pin = boundary
            .add_output_pin("value", "Value", "", VariableType::String)
            .clone();
        layer.pins.insert(pin.id.clone(), pin);
        board.layers.insert(layer.id.clone(), layer);
    }

    fn plan(board: &Board, source: &str, base_module: Option<&str>) -> SignatureMigrations {
        let ast = flow_like_ast::parse(source).expect("valid FlowScript");
        let doc = FlatDocument::new(&ast, base_module);
        let modules = super::super::resolve_modules(board, &doc, base_module);
        assert!(modules.diagnostics.is_empty(), "{:?}", modules.diagnostics);
        SignatureMigrations::plan(board, &doc, &modules, &HashSet::new(), false)
    }

    fn assert_boolean_migration(planned: &SignatureMigrations, layer_id: &str) {
        assert!(planned.diagnostics.is_empty(), "{:?}", planned.diagnostics);
        let commands = planned.commands();
        assert!(
            matches!(commands.as_slice(), [BoardCommand::UpdateFunctionSignature { layer_id: actual, pins, .. }]
                if actual == layer_id && pins.len() == 1
                    && pins[0].name == "value" && pins[0].data_type == "Boolean"),
            "{commands:?}"
        );
    }

    #[test]
    fn unanchored_signature_migration_matches_within_the_written_module() {
        let mut board = board();
        add_module(&mut board, "left-module", "left");
        add_module(&mut board, "right-module", "right");
        add_function(&mut board, "root-helper", "helper", None);
        add_function(&mut board, "left-helper", "helper", Some("left-module"));
        add_function(&mut board, "right-helper", "helper", Some("right-module"));

        let planned = plan(
            &board,
            "module left { function helper(): (value: Boolean) { return true } }",
            None,
        );
        assert_boolean_migration(&planned, "left-helper");
    }

    #[test]
    fn unanchored_signature_migration_normalizes_both_names() {
        let mut board = board();
        add_function(&mut board, "history-row", "psHistoryRow", None);

        let planned = plan(
            &board,
            "function ps_history_row(): (value: Boolean) { return true }",
            None,
        );
        assert_boolean_migration(&planned, "history-row");
    }

    #[test]
    fn unanchored_signature_migration_respects_the_module_file_scope() {
        let mut board = board();
        add_module(&mut board, "history-module", "history");
        add_function(&mut board, "root-helper", "helper", None);
        add_function(
            &mut board,
            "history-helper",
            "helper",
            Some("history-module"),
        );

        let planned = plan(
            &board,
            "function helper(): (value: Boolean) { return true }",
            Some("history-module"),
        );
        assert_boolean_migration(&planned, "history-helper");
    }

    #[test]
    fn unanchored_signature_migration_does_not_claim_a_renamed_layer() {
        let mut board = board();
        add_function(&mut board, "renamed-helper", "helper", None);

        let planned = plan(
            &board,
            "function helper(): (value: Float) { return 1.5 }\n\
             function renamed(): (value: Boolean) { //@l:renamed-helper\n\
                 return true\n\
             }",
            None,
        );
        assert_boolean_migration(&planned, "renamed-helper");
    }

    #[test]
    fn only_anchored_functions_can_migrate_into_a_new_module() {
        let mut board = board();
        add_function(&mut board, "moved-helper", "moved", None);
        add_function(&mut board, "root-helper", "helper", None);

        let planned = plan(
            &board,
            "module created {\n\
                 function helper(): (value: Float) { return 1.5 }\n\
                 function moved(): (value: Boolean) { //@l:moved-helper\n\
                     return true\n\
                 }\n\
             }",
            None,
        );
        assert_boolean_migration(&planned, "moved-helper");
    }

    #[test]
    fn unanchored_signature_migration_does_not_guess_between_duplicate_layers() {
        let mut board = board();
        add_function(&mut board, "first-helper", "helper", None);
        add_function(&mut board, "second-helper", "helper", None);

        let planned = plan(
            &board,
            "function helper(): (value: Boolean) { return true }",
            None,
        );
        assert!(planned.commands().is_empty());
    }
}
