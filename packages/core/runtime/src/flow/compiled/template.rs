//! Shared, immutable run template.
//!
//! A `CompiledRunTemplate` is everything about a board version that does not
//! change between runs: the resolved pin/node topology (index-based, no string
//! probing), the `NodeLogic` singletons, pre-parsed pin/variable defaults, and
//! a Board view for `context.get_board()` consumers. Executors cache one per
//! (board version, registry fingerprint) and construct per-run state from it.
//!
//! The Board view carries `app_state: None` — a template is shared across
//! requests and users, so request-scoped state must never be attached to it.
//! Nothing on the execution path reads `board.app_state` (page loading and
//! `on_update` do, both design-time / API-side).

use super::codes;
use super::format::{CompiledBoard, NONE_IDX};
use super::view::reconstruct_board;
use crate::flow::board::{Board, ExecutionStage, LayerType};
use crate::flow::execution::{ExecutionEnvironment, LogLevel};
use crate::flow::node::{Node, NodeLogic};
use crate::flow::pin::{PinType, ValueType};
use crate::flow::variable::{Variable, VariableType};
use crate::state::FlowNodeRegistryInner;
use ahash::{AHashMap, AHashSet};
use flow_like_storage::Path;
use flow_like_types::{Result, Value, anyhow, sync::Mutex};
use std::sync::Arc;

pub struct TemplatePin {
    pub id: Arc<str>,
    pub name: Arc<str>,
    pub pin_type: PinType,
    pub data_type: VariableType,
    pub value_type: ValueType,
    pub schema: Option<Arc<str>>,
    /// Parsed once at template build; runs share the parsed tree.
    pub default_value: Option<Arc<Value>>,
    pub layer_pin: bool,
    pub sensitive: bool,
    pub index: u16,
    /// Arena index of the owning node — `NONE_IDX` for layer relay pins.
    pub owner_node: u32,
    pub depends_on: Box<[u32]>,
    pub connected_to: Box<[u32]>,
}

pub use crate::flow::execution::internal_node::NodePinLookup;

pub struct TemplateNode {
    /// Shared across runs. Nothing mutates a `Node` during execution — the
    /// mutex exists for API compatibility with `InternalNode.node`.
    pub node: Arc<Mutex<Node>>,
    pub logic: Arc<dyn NodeLogic>,
    /// Trusted catalog restriction, independent of board or artifact metadata.
    pub(crate) only_offline: bool,
    /// This node's pins as arena indices; order matches the lookup offsets.
    pub pins: Box<[u32]>,
    pub lookup: Arc<NodePinLookup>,
    pub is_pure: bool,
    pub id: Arc<str>,
    pub name: Arc<str>,
    /// Top-level nodes retain the historical direct-execution behavior. A
    /// function-layer body node may seed a run only when it is an explicit
    /// entry (`start = true`); ordinary function internals remain callable
    /// only through their layer boundary.
    pub can_seed_run: bool,
    /// An explicit start node inside a Function layer initializes a fresh
    /// Function-local variable scope when selected directly.
    pub seeds_function_scope: bool,
    /// Nearest owning Function layer across both supported board shapes:
    /// legacy nodes physically stored in `Layer.nodes`, and current nodes in
    /// `Board.nodes` carrying a layer tag. Presentational sub-layers inherit
    /// their nearest Function ancestor.
    pub function_layer_id: Option<Arc<str>>,
}

pub struct TemplateVariable {
    /// `value` on this instance is never used; runs clone the struct and
    /// attach a fresh value mutex.
    pub variable: Variable,
    pub parsed_default: Option<Arc<Value>>,
}

/// Function lookups depend only on the compiled topology, so all runs of a
/// template share them. Values still come from each run's own pin arena.
pub(crate) struct FunctionTopology {
    pub(crate) board: Arc<Board>,
    // None marks an ambiguous identifier that retains the runtime scan order.
    pub(crate) runtime_pins: AHashMap<Arc<str>, Option<u32>>,
    pub(crate) definition_owners: AHashMap<String, AHashMap<String, Arc<str>>>,
}

impl FunctionTopology {
    fn new(
        board: Arc<Board>,
        nodes: &[TemplateNode],
        node_idx_by_id: &AHashMap<Arc<str>, u32>,
        pins: &[TemplatePin],
    ) -> Self {
        let mut runtime_pins = AHashMap::with_capacity(pins.len());
        for (node_idx, node) in nodes.iter().enumerate() {
            // InternalRun's node map retains the final node with a given ID.
            if node_idx_by_id.get(node.id.as_ref()) != Some(&(node_idx as u32)) {
                continue;
            }
            for (id, &offset) in &node.lookup.by_id {
                let arena_idx = node.pins[offset as usize];
                let pin = &pins[arena_idx as usize];
                let id = if pin.id.as_ref() == id {
                    pin.id.clone()
                } else {
                    Arc::from(id.as_str())
                };
                runtime_pins
                    .entry(id)
                    .and_modify(|existing| {
                        if *existing != Some(arena_idx) {
                            *existing = None;
                        }
                    })
                    .or_insert(Some(arena_idx));
            }
        }

        let mut definition_owners = AHashMap::with_capacity(board.layers.len());
        for (id, layer) in &board.layers {
            let mut owners = AHashMap::new();
            for node in layer.nodes.values() {
                let owner: Arc<str> = Arc::from(node.id.as_str());
                for pin_id in node.pins.keys() {
                    owners
                        .entry(pin_id.clone())
                        .or_insert_with(|| owner.clone());
                }
            }
            definition_owners.insert(id.clone(), owners);
        }
        // Legacy layouts can keep function nodes in Board.nodes. The layer's
        // own definitions take precedence, including duplicate identifiers.
        for node in board.nodes.values() {
            let Some(owners) = node
                .layer
                .as_ref()
                .and_then(|id| definition_owners.get_mut(id))
            else {
                continue;
            };
            let owner: Arc<str> = Arc::from(node.id.as_str());
            for pin_id in node.pins.keys() {
                owners
                    .entry(pin_id.clone())
                    .or_insert_with(|| owner.clone());
            }
        }
        Self {
            board,
            runtime_pins,
            definition_owners,
        }
    }
}

pub struct CompiledRunTemplate {
    pub widget_snapshot: Option<Arc<super::ArtifactWidgetSnapshot>>,
    pub pins: Box<[TemplatePin]>,
    pub nodes: Box<[TemplateNode]>,
    pub node_idx_by_id: AHashMap<Arc<str>, u32>,
    pub variables: Box<[TemplateVariable]>,
    /// Execution-scoped Board view (`context.get_board()`, layer/ref lookups).
    pub board: Arc<Board>,
    pub stage: ExecutionStage,
    pub log_level: LogLevel,
    pub(crate) function_topology: Arc<FunctionTopology>,
}

impl CompiledRunTemplate {
    /// Validate every resolved node, including function bodies, against the
    /// host environment before a cached template is used for a run.
    pub fn ensure_execution_environment(&self, environment: ExecutionEnvironment) -> Result<()> {
        if environment != ExecutionEnvironment::Server {
            return Ok(());
        }
        for node in self.nodes.iter() {
            environment.ensure_node_execution(&node.name, node.only_offline)?;
        }
        Ok(())
    }

    /// Build a template from a fully prepared board (post `node_updates`).
    pub fn from_board(board: Arc<Board>, registry: &FlowNodeRegistryInner) -> Result<Self> {
        let compiled = super::compile::compile_board_with_catalog(&board, registry)?;
        Self::from_compiled(&compiled, registry, board.board_dir.clone())
    }

    /// Build a template from a compiled board. The Board view is always
    /// reconstructed from the compiled form — never the source board — so the
    /// view's wiring matches the executed (reroute-spliced) graph on every
    /// path. Nodes like call_function navigate the view's edges and resolve
    /// them against `context.nodes`; a view with unspliced edges would point
    /// at pins that no longer exist in the run graph.
    pub fn from_compiled(
        compiled: &CompiledBoard,
        registry: &FlowNodeRegistryInner,
        board_dir: Path,
    ) -> Result<Self> {
        let view = Arc::new(reconstruct_board(compiled, board_dir, Some(registry))?);
        Self::build(compiled, registry, view)
    }

    fn build(
        compiled: &CompiledBoard,
        registry: &FlowNodeRegistryInner,
        view: Arc<Board>,
    ) -> Result<Self> {
        let mut pins = Vec::with_capacity(compiled.pins.len());
        for cp in &compiled.pins {
            let data_type = codes::variable_type_from(cp.data_type)?;
            let value_type = codes::value_type_from(cp.value_type)?;
            let schema = if data_type == VariableType::Geometry {
                cp.schema
                    .as_deref()
                    .map(|schema| crate::flow::pin::resolve_schema(schema, &view.refs))
                    .transpose()?
            } else {
                cp.schema.as_deref()
            };
            crate::flow::variable::validate_typed_default(
                &data_type,
                &value_type,
                schema,
                cp.default_value.as_deref(),
            )?;
            let default_value = cp
                .default_value
                .as_ref()
                .and_then(|bytes| flow_like_types::json::from_slice::<Value>(bytes).ok())
                .filter(|value| data_type != VariableType::Geometry || !value.is_null())
                .map(Arc::new);
            pins.push(TemplatePin {
                id: Arc::from(cp.id.as_str()),
                name: Arc::from(cp.name.as_str()),
                pin_type: codes::pin_type_from(cp.pin_type)?,
                data_type,
                value_type,
                schema: schema.map(Arc::from),
                default_value,
                layer_pin: cp.owner_node == NONE_IDX && cp.owner_layer != NONE_IDX,
                sensitive: cp
                    .options
                    .as_ref()
                    .and_then(|options| options.sensitive)
                    .unwrap_or(false),
                index: cp.index,
                owner_node: cp.owner_node,
                depends_on: cp.depends_on.clone().into_boxed_slice(),
                connected_to: cp.connected_to.clone().into_boxed_slice(),
            });
        }

        let layer_ids: Vec<&str> = compiled.layers.iter().map(|l| l.id.as_str()).collect();
        // Nodes in the same layer share their nearest Function ancestor.
        // Resolve it once per layer instead of allocating a traversal per node.
        let function_layers: Vec<Option<Arc<str>>> = (0..compiled.layers.len())
            .map(|idx| {
                owning_function_layer_idx(compiled, idx as u32)
                    .map(|owner| Arc::from(compiled.layers[owner as usize].id.as_str()))
            })
            .collect();

        let mut nodes = Vec::with_capacity(compiled.nodes.len());
        let mut node_idx_by_id: AHashMap<Arc<str>, u32> =
            AHashMap::with_capacity(compiled.nodes.len());
        for (i, cn) in compiled.nodes.iter().enumerate() {
            let layer_idx = if cn.body_layer != NONE_IDX {
                cn.body_layer
            } else {
                cn.layer
            };
            let function_layer_id = function_layers.get(layer_idx as usize).cloned().flatten();
            let source = if cn.body_layer == NONE_IDX {
                view.nodes.get(&cn.id)
            } else {
                layer_ids
                    .get(cn.body_layer as usize)
                    .and_then(|layer_id| view.layers.get(*layer_id))
                    .and_then(|layer| layer.nodes.get(&cn.id))
            };
            let node = source
                .ok_or_else(|| {
                    anyhow!(
                        "compiled node {} ({}) missing from the board view",
                        cn.id,
                        cn.name
                    )
                })?
                .clone();
            let logic = registry.instantiate(&node)?;
            let only_offline = registry.get_node_ref(&node.name)?.only_offline;

            let mut lookup = NodePinLookup {
                by_id: AHashMap::with_capacity(cn.pins.len()),
                by_name: AHashMap::with_capacity(cn.pins.len()),
            };
            for (offset, &arena_idx) in cn.pins.iter().enumerate() {
                let pin = &pins[arena_idx as usize];
                lookup.by_id.insert(pin.id.to_string(), offset as u16);
                lookup
                    .by_name
                    .entry(pin.name.to_string())
                    .or_default()
                    .push(offset as u16);
            }

            let is_pure = node.is_pure();
            let id: Arc<str> = Arc::from(cn.id.as_str());
            node_idx_by_id.insert(id.clone(), i as u32);
            nodes.push(TemplateNode {
                node: Arc::new(Mutex::new(node)),
                logic,
                only_offline,
                pins: cn.pins.clone().into_boxed_slice(),
                lookup: Arc::new(lookup),
                is_pure,
                id,
                name: Arc::from(cn.name.as_str()),
                // Nodes stored in Board.nodes were historically valid direct
                // executeBoard targets even when tagged into a Function layer.
                // Preserve that path, while `start` additionally admits legacy
                // nodes stored inside Layer.nodes for governed Page events.
                can_seed_run: cn.body_layer == NONE_IDX || cn.start,
                seeds_function_scope: cn.start && function_layer_id.is_some(),
                function_layer_id,
            });
        }

        let mut variables = Vec::with_capacity(view.variables.len());
        for variable in view.variables.values() {
            let mut variable = variable.clone();
            if variable.data_type == VariableType::Geometry {
                variable.schema = variable
                    .schema
                    .as_deref()
                    .map(|schema| {
                        crate::flow::pin::resolve_schema(schema, &view.refs).map(str::to_string)
                    })
                    .transpose()?;
                crate::flow::variable::validate_typed_default(
                    &variable.data_type,
                    &variable.value_type,
                    variable.schema.as_deref(),
                    variable.default_value.as_deref(),
                )?;
            }
            variables.push(TemplateVariable {
                parsed_default: variable
                    .default_value
                    .as_ref()
                    .and_then(|bytes| flow_like_types::json::from_slice::<Value>(bytes).ok())
                    .map(Arc::new),
                variable,
            });
        }

        let function_topology = Arc::new(FunctionTopology::new(
            view.clone(),
            &nodes,
            &node_idx_by_id,
            &pins,
        ));
        Ok(Self {
            widget_snapshot: None,
            pins: pins.into_boxed_slice(),
            nodes: nodes.into_boxed_slice(),
            node_idx_by_id,
            variables: variables.into_boxed_slice(),
            stage: view.stage.clone(),
            log_level: view.log_level,
            board: view,
            function_topology,
        })
    }
}

fn owning_function_layer_idx(compiled: &CompiledBoard, layer_idx: u32) -> Option<u32> {
    let mut current = Some(layer_idx);
    let mut seen = AHashSet::with_capacity(4);

    while let Some(idx) = current {
        if !seen.insert(idx) {
            return None;
        }
        let layer = compiled.layers.get(idx as usize)?;
        if matches!(
            codes::layer_type_from(layer.layer_type),
            Ok(LayerType::Function)
        ) {
            return Some(idx);
        }
        current = (layer.parent_layer != NONE_IDX).then_some(layer.parent_layer);
    }

    None
}

#[cfg(test)]
mod catalog_policy_tests {
    use super::*;
    use crate::flow::board::Layer;
    use crate::flow::compiled::compile::compile_board;
    use crate::flow::execution::context::ExecutionContext;
    use flow_like_types::async_trait;

    struct CatalogLogic;

    #[async_trait]
    impl NodeLogic for CatalogLogic {
        fn get_node(&self) -> Node {
            Node::new("catalog_policy", "Policy", "", "Tests")
        }

        async fn run(&self, _: &mut ExecutionContext) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn function_ownership_keeps_nested_layers_cycles_and_legacy_bodies() {
        let mut board = Board::new_detached(None, Path::default());
        for (id, kind, parent) in [
            ("outer", LayerType::Function, None),
            ("child", LayerType::Collapsed, Some("outer")),
            ("inner", LayerType::Function, Some("child")),
            ("grandchild", LayerType::Collapsed, Some("inner")),
            ("orphan", LayerType::Collapsed, None),
            ("cycle-a", LayerType::Collapsed, Some("cycle-b")),
            ("cycle-b", LayerType::Collapsed, Some("cycle-a")),
        ] {
            let mut layer = Layer::new(id.into(), id.into(), kind);
            layer.parent_id = parent.map(str::to_string);
            board.layers.insert(layer.id.clone(), layer);
        }
        let tagged = [
            ("root", None, true, None),
            ("in-child", Some("child"), true, Some("outer")),
            ("in-inner", Some("inner"), true, Some("inner")),
            ("in-grandchild", Some("grandchild"), true, Some("inner")),
            ("internal", Some("grandchild"), false, Some("inner")),
            ("in-orphan", Some("orphan"), true, None),
            ("in-cycle", Some("cycle-a"), true, None),
        ];
        for (id, layer, start, _) in tagged {
            let mut node = CatalogLogic.get_node();
            node.id = id.into();
            node.layer = layer.map(str::to_string);
            node.set_start(start);
            board.nodes.insert(node.id.clone(), node);
        }
        for (id, start) in [("legacy-entry", true), ("legacy-internal", false)] {
            let mut node = CatalogLogic.get_node();
            node.id = id.into();
            node.layer = Some("inner".into());
            node.set_start(start);
            board
                .layers
                .get_mut("outer")
                .unwrap()
                .nodes
                .insert(node.id.clone(), node);
        }
        let mut registry = FlowNodeRegistryInner::new(1);
        registry.insert(CatalogLogic.get_node(), Arc::new(CatalogLogic));
        let compiled = compile_board(&board).unwrap();
        let bytes = super::super::encode_artifact(&compiled, &registry.fingerprint()).unwrap();
        let template = super::super::template_from_bytes(
            &bytes,
            &registry.fingerprint(),
            &registry,
            &Path::default(),
        )
        .unwrap();
        for (id, _, start, owner) in tagged {
            let node = &template.nodes[template.node_idx_by_id[id] as usize];
            assert_eq!(node.function_layer_id.as_deref(), owner, "{id}");
            assert_eq!(node.seeds_function_scope, start && owner.is_some(), "{id}");
            assert!(
                node.can_seed_run,
                "top-level node {id} retains direct execution"
            );
        }
        for (id, start) in [("legacy-entry", true), ("legacy-internal", false)] {
            let node = &template.nodes[template.node_idx_by_id[id] as usize];
            assert_eq!(node.function_layer_id.as_deref(), Some("outer"));
            assert_eq!(node.seeds_function_scope, start);
            assert_eq!(node.can_seed_run, start);
        }
    }

    #[test]
    fn borrowed_catalog_policy_overrides_forged_artifact_flags_in_both_directions() {
        for only_offline in [false, true] {
            let mut catalog_node = CatalogLogic.get_node();
            catalog_node.only_offline = only_offline;
            let mut placed = catalog_node.clone();
            placed.only_offline = !only_offline;
            let mut board = Board::new_detached(None, Path::default());
            board.nodes.insert(placed.id.clone(), placed);
            let compiled = compile_board(&board).unwrap();
            assert_eq!(compiled.nodes[0].only_offline, !only_offline);

            let mut registry = FlowNodeRegistryInner::new(1);
            registry.insert(catalog_node, Arc::new(CatalogLogic));
            let direct = Arc::new(
                CompiledRunTemplate::from_compiled(&compiled, &registry, Path::default()).unwrap(),
            );
            let bytes = super::super::encode_artifact(&compiled, &registry.fingerprint()).unwrap();
            let artifact = super::super::template_from_bytes(
                &bytes,
                &registry.fingerprint(),
                &registry,
                &Path::default(),
            )
            .unwrap();
            for template in [direct, artifact] {
                assert_eq!(template.nodes[0].only_offline, only_offline);
                assert_eq!(
                    template
                        .ensure_execution_environment(ExecutionEnvironment::Server)
                        .is_err(),
                    only_offline,
                );
                for environment in [
                    ExecutionEnvironment::Local,
                    ExecutionEnvironment::Desktop,
                    ExecutionEnvironment::Mobile,
                    ExecutionEnvironment::BrowserSandbox,
                ] {
                    template.ensure_execution_environment(environment).unwrap();
                }
            }
        }
    }

    #[test]
    fn missing_catalog_policy_keeps_the_instantiation_error() {
        let mut board = Board::new_detached(None, Path::default());
        let node = CatalogLogic.get_node();
        let expected = format!(
            "Node not found - Instancing: '{}' (id: {}, category: {})",
            node.name, node.id, node.category
        );
        board.nodes.insert(node.id.clone(), node);
        let compiled = compile_board(&board).unwrap();
        let error = CompiledRunTemplate::from_compiled(
            &compiled,
            &FlowNodeRegistryInner::new(0),
            Path::default(),
        )
        .err()
        .expect("an artifact cannot supply its own missing catalog authority");
        assert_eq!(error.to_string(), expected);
    }
}
