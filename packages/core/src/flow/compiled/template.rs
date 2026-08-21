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
use crate::flow::board::{Board, ExecutionStage};
use crate::flow::execution::LogLevel;
use crate::flow::node::{Node, NodeLogic};
use crate::flow::pin::PinType;
use crate::flow::variable::{Variable, VariableType};
use crate::state::FlowNodeRegistryInner;
use ahash::AHashMap;
use flow_like_storage::Path;
use flow_like_types::{Result, Value, anyhow, sync::Mutex};
use std::sync::Arc;

pub struct TemplatePin {
    pub id: Arc<str>,
    pub name: Arc<str>,
    pub pin_type: PinType,
    pub data_type: VariableType,
    /// Parsed once at template build; runs share the parsed tree.
    pub default_value: Option<Arc<Value>>,
    pub layer_pin: bool,
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
    /// This node's pins as arena indices; order matches the lookup offsets.
    pub pins: Box<[u32]>,
    pub lookup: Arc<NodePinLookup>,
    pub is_pure: bool,
    pub id: Arc<str>,
    pub name: Arc<str>,
    /// Nodes inside a function-layer body cannot seed the run stack.
    pub in_layer_body: bool,
}

pub struct TemplateVariable {
    /// `value` on this instance is never used; runs clone the struct and
    /// attach a fresh value mutex.
    pub variable: Variable,
    pub parsed_default: Option<Arc<Value>>,
}

pub struct CompiledRunTemplate {
    pub pins: Box<[TemplatePin]>,
    pub nodes: Box<[TemplateNode]>,
    pub node_idx_by_id: AHashMap<Arc<str>, u32>,
    pub variables: Box<[TemplateVariable]>,
    /// Execution-scoped Board view (`context.get_board()`, layer/ref lookups).
    pub board: Arc<Board>,
    pub stage: ExecutionStage,
    pub log_level: LogLevel,
}

impl CompiledRunTemplate {
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
            pins.push(TemplatePin {
                id: Arc::from(cp.id.as_str()),
                name: Arc::from(cp.name.as_str()),
                pin_type: codes::pin_type_from(cp.pin_type)?,
                data_type: codes::variable_type_from(cp.data_type)?,
                default_value: cp
                    .default_value
                    .as_ref()
                    .and_then(|bytes| flow_like_types::json::from_slice::<Value>(bytes).ok())
                    .map(Arc::new),
                layer_pin: cp.owner_node == NONE_IDX && cp.owner_layer != NONE_IDX,
                index: cp.index,
                owner_node: cp.owner_node,
                depends_on: cp.depends_on.clone().into_boxed_slice(),
                connected_to: cp.connected_to.clone().into_boxed_slice(),
            });
        }

        let layer_ids: Vec<&str> = compiled.layers.iter().map(|l| l.id.as_str()).collect();

        let mut nodes = Vec::with_capacity(compiled.nodes.len());
        let mut node_idx_by_id: AHashMap<Arc<str>, u32> =
            AHashMap::with_capacity(compiled.nodes.len());
        for (i, cn) in compiled.nodes.iter().enumerate() {
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

            let mut lookup = NodePinLookup::default();
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
                pins: cn.pins.clone().into_boxed_slice(),
                lookup: Arc::new(lookup),
                is_pure,
                id,
                name: Arc::from(cn.name.as_str()),
                in_layer_body: cn.body_layer != NONE_IDX,
            });
        }

        let mut variables = Vec::with_capacity(view.variables.len());
        for variable in view.variables.values() {
            variables.push(TemplateVariable {
                parsed_default: variable
                    .default_value
                    .as_ref()
                    .and_then(|bytes| flow_like_types::json::from_slice::<Value>(bytes).ok())
                    .map(Arc::new),
                variable: variable.clone(),
            });
        }

        Ok(Self {
            pins: pins.into_boxed_slice(),
            nodes: nodes.into_boxed_slice(),
            node_idx_by_id,
            variables: variables.into_boxed_slice(),
            stage: view.stage.clone(),
            log_level: view.log_level,
            board: view,
        })
    }
}
