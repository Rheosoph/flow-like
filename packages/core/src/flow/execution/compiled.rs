//! The hydrated form of a compiled execution plan.
//!
//! A [`CompiledGraph`] is the immutable half of a run: topology, resolved node logic and
//! pre-parsed default values. It is built once per (board, version, catalog) and shared by
//! every run of that board, so all the work the old builder repeated per run — registry
//! lookups by string, JSON parsing of every default, deep-cloning every `Node` — happens
//! here at most once.
//!
//! What stays per-run is only mutable state: pin values, variable values and the trace
//! machinery. That split is what makes starting a run cheap.
//!
//! The COLD section is deliberately not touched during hydration. Its strings are needed
//! only by LLM/MCP/REST/form nodes, so a flow that never reaches one never pays to
//! validate or decompress them.

use ahash::AHashMap;
use std::sync::{Arc, OnceLock};

use flow_like_types::{
    Value,
    sync::Mutex,
    plan::{
        ArchivedColdPlan, ArchivedHotPlan, AlignedSection, NONE_INDEX, PlanBuffer, PlanError,
        header::PlanStamps,
    },
};

use crate::{
    flow::{
        board::Board,
        execution::{
            LogLevel,
            internal_node::{InternalNode, NodeMeta},
            internal_pin::InternalPin,
        },
        node::{FnRefs, Node, NodeLogic, NodeWasm},
        pin::{Pin, PinOptions, PinType, ValueType},
        variable::VariableType,
    },
    state::FlowNodeRegistryInner,
};

#[derive(Debug, thiserror::Error)]
pub enum HydrateError {
    #[error("plan is unreadable: {0}")]
    Plan(#[from] PlanError),
    #[error("plan references node type {type_key} which the registry cannot resolve")]
    UnknownNodeType { type_key: String },
    #[error("plan holds an out-of-range {kind} value {value}")]
    InvalidEnum { kind: &'static str, value: u8 },
}

/// Lock-free per-node facts the scheduler and logger need on every step.
///
/// Mirrors the existing `NodeMeta`: these are exactly the fields the hot path reads, so it
/// never has to materialize a full [`Node`].
#[derive(Debug, Clone)]
pub struct CompiledNodeMeta {
    pub id: Arc<str>,
    pub name: Arc<str>,
    pub is_pure: bool,
}

/// An immutable execution skeleton, shared across runs.
pub struct CompiledGraph {
    buffer: PlanBuffer,
    hot: AlignedSection<ArchivedHotPlan>,
    cold: OnceLock<Option<AlignedSection<ArchivedColdPlan>>>,

    logic: Box<[Arc<dyn NodeLogic>]>,
    meta: Box<[CompiledNodeMeta]>,
    /// Editor-shaped node definitions, materialized on first use and then shared by every
    /// run. Read-only during execution, which is what makes sharing sound.
    nodes: Box<[OnceLock<Arc<Mutex<Node>>>]>,
    /// Blob index -> parsed default. Parsed once here instead of per pin per run.
    defaults: Box<[Arc<Value>]>,
    /// Runtime-shaped board projected from the plan, built on first use and shared by
    /// every run — so no run ever loads the `.board` object.
    board: OnceLock<Arc<Board>>,
}

impl std::fmt::Debug for CompiledGraph {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompiledGraph")
            .field("nodes", &self.meta.len())
            .field("stamps", &self.stamps())
            .finish_non_exhaustive()
    }
}

impl CompiledGraph {
    /// Validate the HOT section and resolve everything a run needs up front.
    pub fn hydrate(
        buffer: PlanBuffer,
        registry: &FlowNodeRegistryInner,
    ) -> Result<Self, HydrateError> {
        let hot = buffer.hot()?;

        let (logic, meta, defaults) = {
            let plan = hot.root();

            let mut logic = Vec::with_capacity(plan.nodes.len());
            let mut meta = Vec::with_capacity(plan.nodes.len());

            for entry in plan.nodes.iter() {
                let type_key = plan.symbol(entry.type_key.to_native());
                let resolved = registry.instantiate_by_name(type_key).ok_or_else(|| {
                    HydrateError::UnknownNodeType {
                        type_key: type_key.to_string(),
                    }
                })?;

                logic.push(resolved);
                meta.push(CompiledNodeMeta {
                    id: Arc::from(plan.symbol(entry.instance_id.to_native())),
                    name: Arc::from(type_key),
                    is_pure: entry.flags.to_native()
                        & flow_like_types::plan::hot::node_flags::IS_PURE
                        != 0,
                });
            }

            // Defaults are canonical JSON; an unparseable blob degrades to `Null`, which is
            // exactly what the runtime does today rather than failing the run.
            let defaults: Vec<Arc<Value>> = plan
                .blobs
                .iter()
                .map(|blob| {
                    Arc::new(
                        flow_like_types::json::from_slice::<Value>(blob).unwrap_or(Value::Null),
                    )
                })
                .collect();

            (logic, meta, defaults)
        };

        let nodes = (0..meta.len()).map(|_| OnceLock::new()).collect();

        Ok(Self {
            buffer,
            hot,
            cold: OnceLock::new(),
            logic: logic.into_boxed_slice(),
            meta: meta.into_boxed_slice(),
            nodes,
            defaults: defaults.into_boxed_slice(),
            board: OnceLock::new(),
        })
    }

    pub fn plan(&self) -> &ArchivedHotPlan {
        self.hot.root()
    }

    pub fn stamps(&self) -> PlanStamps {
        self.buffer.header().stamps
    }

    pub fn node_count(&self) -> usize {
        self.meta.len()
    }

    pub fn meta(&self, node: u32) -> Option<&CompiledNodeMeta> {
        self.meta.get(node as usize)
    }

    pub fn logic(&self, node: u32) -> Option<&Arc<dyn NodeLogic>> {
        self.logic.get(node as usize)
    }

    pub fn node_by_id(&self, id: &str) -> Option<u32> {
        self.plan().node_by_id(id)
    }

    /// Pre-parsed default for a pin, or `None` when it has none.
    pub fn pin_default(&self, pin: u32) -> Option<Arc<Value>> {
        let plan = self.plan();
        let entry = plan.pins.get(pin as usize)?;
        let blob = entry.default_value.to_native();
        (blob != NONE_INDEX).then(|| self.defaults[blob as usize].clone())
    }

    /// Validate the COLD section on first use.
    ///
    /// Returns `None` when the plan carries no COLD section at all, which is legal — the
    /// callers that need it (LLM tooling, MCP, REST, forms) degrade to empty strings the
    /// same way they do for a node with no description.
    pub fn cold(&self) -> Option<&ArchivedColdPlan> {
        self.cold
            .get_or_init(|| {
                if !self.buffer.has_cold() {
                    return None;
                }
                match self.buffer.cold() {
                    Ok(section) => Some(section),
                    Err(error) => {
                        tracing::warn!("plan cold section unusable: {error}");
                        None
                    }
                }
            })
            .as_ref()
            .map(|section| section.root())
    }

    /// The editor-shaped [`Node`] for a plan index, materialized on demand.
    ///
    /// Roughly forty catalog sites reach for the full node to read display metadata or pin
    /// shape. Rebuilding it from HOT + COLD keeps those working unchanged, while the hot
    /// path stays on [`CompiledGraph::meta`] and never triggers this.
    pub fn node(&self, index: u32) -> Option<Arc<Mutex<Node>>> {
        let slot = self.nodes.get(index as usize)?;
        if let Some(node) = slot.get() {
            return Some(node.clone());
        }
        let built = Arc::new(Mutex::new(self.materialize_node(index)?));
        Some(slot.get_or_init(|| built).clone())
    }

    fn materialize_node(&self, index: u32) -> Option<Node> {
        let plan = self.plan();
        let entry = plan.nodes.get(index as usize)?;
        let cold = self.cold();

        let meta = &self.meta[index as usize];
        let mut node = Node::new(
            &meta.name,
            cold.map(|c| c.node_friendly_name(index)).unwrap_or(""),
            cold.map(|c| c.node_description(index)).unwrap_or(""),
            "",
        );
        node.id = meta.id.to_string();
        // Carries the compile-time hash so runtime cache keys stay stable without the
        // recompute that used to force layout fields into the artifact.
        node.hash = Some(entry.semantic_hash.to_native());

        let layer = entry.layer.to_native();
        if layer != NONE_INDEX {
            if let Some(plan_layer) = plan.layers.get(layer as usize) {
                node.layer = Some(plan.symbol(plan_layer.id.to_native()).to_string());
            }
        }

        let wasm = entry.wasm_package.to_native();
        if wasm != NONE_INDEX {
            node.wasm = Some(NodeWasm {
                package_id: plan.symbol(wasm).to_string(),
                // Unknown encodings are dropped rather than guessed: a permission must
                // fail closed.
                permissions: plan
                    .wasm_permissions
                    .row(index as usize)
                    .iter()
                    .filter_map(|value| {
                        crate::flow::node::NodePermission::from_plan_u8(value.to_native() as u8)
                    })
                    .collect(),
            });
        }

        let flags = entry.flags.to_native();
        let can_reference = flags & flow_like_types::plan::hot::node_flags::CAN_REFERENCE_FNS != 0;
        let can_be_referenced =
            flags & flow_like_types::plan::hot::node_flags::CAN_BE_REFERENCED_BY_FNS != 0;
        let refs = plan.fn_refs.row(index as usize);
        if can_reference || can_be_referenced || !refs.is_empty() {
            node.fn_refs = Some(FnRefs {
                fn_refs: refs
                    .iter()
                    .map(|symbol| plan.symbol(symbol.to_native()).to_string())
                    .collect(),
                can_reference_fns: can_reference,
                can_be_referenced_by_fns: can_be_referenced,
            });
        }

        let first = entry.first_pin.to_native();
        for offset in 0..entry.pin_count.to_native() {
            let pin_index = first + offset;
            let Some(pin) = self.materialize_pin(pin_index) else {
                continue;
            };
            node.pins.insert(pin.id.clone(), pin);
        }

        Some(node)
    }

    fn materialize_pin(&self, index: u32) -> Option<Pin> {
        let plan = self.plan();
        let entry = plan.pins.get(index as usize)?;
        let cold = self.cold();

        let valid_values: Vec<String> = cold
            .map(|c| {
                c.pin_valid_values(index)
                    .into_iter()
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();

        Some(Pin {
            id: plan.symbol(entry.id.to_native()).to_string(),
            name: plan.symbol(entry.name.to_native()).to_string(),
            friendly_name: cold
                .map(|c| c.pin_friendly_name(index))
                .unwrap_or("")
                .to_string(),
            description: cold
                .map(|c| c.pin_description(index))
                .unwrap_or("")
                .to_string(),
            pin_type: PinType::from_plan_u8(entry.pin_type)?,
            data_type: VariableType::from_plan_u8(entry.data_type)?,
            schema: cold
                .and_then(|c| c.pin_schema(index))
                .map(str::to_string),
            value_type: ValueType::from_plan_u8(entry.value_type)?,
            // Edges live in the plan's CSR tables; the reconstructed pin carries the same
            // information for the catalog sites that inspect it.
            depends_on: plan
                .pin_dependencies
                .row(index as usize)
                .iter()
                .filter_map(|source| self.pin_id(source.to_native()))
                .collect(),
            connected_to: plan
                .pin_connections
                .row(index as usize)
                .iter()
                .filter_map(|target| self.pin_id(target.to_native()))
                .collect(),
            default_value: {
                let blob = entry.default_value.to_native();
                (blob != NONE_INDEX).then(|| plan.blobs[blob as usize].to_vec())
            },
            index: entry.index.to_native(),
            options: (!valid_values.is_empty()).then(|| PinOptions {
                valid_values: Some(valid_values),
                ..Default::default()
            }),
            value: None,
        })
    }

    fn pin_id(&self, index: u32) -> Option<String> {
        let plan = self.plan();
        plan.pins
            .get(index as usize)
            .map(|pin| plan.symbol(pin.id.to_native()).to_string())
    }

    /// Materialize a runtime-shaped [`Board`] from the plan, so a run never has to load
    /// the `.board` object at all.
    ///
    /// This is what removes the last dependency on the source board and unlocks the cold
    /// -load win: instead of an object-store GET plus lz4/protobuf decode plus the
    /// `node_updates` fixpoint plus `cleanup`, the board a run needs is projected from the
    /// already-validated plan.
    ///
    /// Only what the runtime actually reads is reproduced — an audit of every
    /// `get_board()`/`run.board` consumer in the catalog found exactly two uses:
    /// `layers.get(id)` for `{type, name, variables}` (function invocation and variable
    /// isolation in `call_function`, `call_ref`, agent helpers) and `refs`. Editor-only
    /// state (comments, viewport, coordinates, timestamps) is deliberately absent.
    ///
    /// `refs` is intentionally empty: the compiler already resolved every ref-hash into the
    /// literal string stored in the cold section, and the runtime's `resolve_ref` falls
    /// back to the value itself when a key is absent — so an empty table yields exactly the
    /// resolved text.
    pub fn board(&self, board_dir: &flow_like_storage::Path) -> Arc<Board> {
        if let Some(board) = self.board.get() {
            return board.clone();
        }
        let built = Arc::new(self.materialize_board(board_dir));
        self.board.get_or_init(|| built).clone()
    }

    fn materialize_board(&self, board_dir: &flow_like_storage::Path) -> Board {
        use crate::flow::board::{Layer, LayerType};

        let plan = self.plan();
        let cold = self.cold();
        let mut board = Board::new_detached(Some(plan.board_id.to_string()), board_dir.clone());

        board.stage = crate::flow::board::ExecutionStage::from_plan_u8(plan.stage)
            .unwrap_or(crate::flow::board::ExecutionStage::Dev);
        board.log_level = LogLevel::from_u8(plan.log_level);
        board.version = self.stamps().board_version;
        // Left empty on purpose: refs were inlined at compile time (see doc above).
        board.refs = std::collections::HashMap::new();

        // Built directly rather than through `node()`, which hands back the shared
        // `Arc<Mutex<Node>>` the runtime graph uses; the board wants owned definitions.
        for index in 0..plan.nodes.len() as u32 {
            if let Some(node) = self.materialize_node(index) {
                board.nodes.insert(node.id.clone(), node);
            }
        }

        for (index, plan_layer) in plan.layers.iter().enumerate() {
            let id = plan.symbol(plan_layer.id.to_native()).to_string();
            let mut layer = Layer::new(
                id.clone(),
                cold.map(|c| c.layer_name(index as u32)).unwrap_or("").to_string(),
                match plan_layer.layer_type {
                    0 => LayerType::Function,
                    1 => LayerType::Macro,
                    _ => LayerType::Collapsed,
                },
            );
            let parent = plan_layer.parent.to_native();
            if parent != NONE_INDEX {
                if let Some(parent_layer) = plan.layers.get(parent as usize) {
                    layer.parent_id = Some(plan.symbol(parent_layer.id.to_native()).to_string());
                }
            }
            for variable_index in plan.layer_variables.row(index) {
                if let Some(variable) = self.variable(variable_index.to_native()) {
                    layer.variables.insert(variable.id.clone(), variable);
                }
            }
            board.layers.insert(id, layer);
        }

        for index in 0..plan.variables.len() as u32 {
            let entry = &plan.variables[index as usize];
            if entry.owner_layer.to_native() != NONE_INDEX {
                continue;
            }
            if let Some(variable) = self.variable(index) {
                board.variables.insert(variable.id.clone(), variable);
            }
        }

        board
    }

    /// Rebuild a [`Variable`] from its plan entry.
    fn variable(&self, index: u32) -> Option<crate::flow::variable::Variable> {
        use crate::flow::variable::Variable;
        use flow_like_types::plan::hot::variable_flags;

        let plan = self.plan();
        let entry = plan.variables.get(index as usize)?;
        let flags = entry.flags;

        let mut variable = Variable::new(
            plan.symbol(entry.name.to_native()),
            VariableType::from_plan_u8(entry.data_type)?,
            ValueType::from_plan_u8(entry.value_type)?,
        );
        variable.id = plan.symbol(entry.id.to_native()).to_string();
        variable.exposed = flags & variable_flags::EXPOSED != 0;
        variable.secret = flags & variable_flags::SECRET != 0;
        variable.editable = flags & variable_flags::EDITABLE != 0;
        variable.runtime_configured = flags & variable_flags::RUNTIME_CONFIGURED != 0;

        let blob = entry.default_value.to_native();
        variable.default_value =
            (blob != NONE_INDEX).then(|| plan.blobs[blob as usize].to_vec());

        Some(variable)
    }

    /// Build the per-run runtime graph.
    ///
    /// This replaces the old builder's six string-keyed passes over pins and two over
    /// nodes. Edges are already `u32` indices into a flat arena, so wiring is a slice walk
    /// with no hashing, and pin defaults come from the pre-parsed table instead of a
    /// `serde_json` call per pin per run.
    pub fn build_runtime_graph(&self) -> Result<RuntimeGraph, HydrateError> {
        let plan = self.plan();

        // Pass 1: allocate every pin. Indices in the plan line up with this vector, so the
        // wiring pass below can address pins positionally.
        let pins: Vec<Arc<InternalPin>> = (0..plan.pins.len() as u32)
            .map(|index| {
                let entry = &plan.pins[index as usize];
                Ok(Arc::new(InternalPin::from_parts(
                    plan.symbol(entry.id.to_native()).to_string(),
                    plan.symbol(entry.name.to_native()).to_string(),
                    PinType::from_plan_u8(entry.pin_type).ok_or(HydrateError::InvalidEnum {
                        kind: "PinType",
                        value: entry.pin_type,
                    })?,
                    VariableType::from_plan_u8(entry.data_type).ok_or(
                        HydrateError::InvalidEnum {
                            kind: "VariableType",
                            value: entry.data_type,
                        },
                    )?,
                    self.pin_default(index).map(|value| (*value).clone()),
                    entry.flags.to_native() & flow_like_types::plan::hot::pin_flags::IS_LAYER_PIN
                        != 0,
                    entry.index.to_native(),
                )))
            })
            .collect::<Result<_, HydrateError>>()?;

        // Pass 2: wire edges straight from the CSR rows.
        for (index, pin) in pins.iter().enumerate() {
            pin.init_connected_to(
                plan.pin_connections
                    .row(index)
                    .iter()
                    .map(|target| Arc::downgrade(&pins[target.to_native() as usize]))
                    .collect(),
            );
            pin.init_depends_on(
                plan.pin_dependencies
                    .row(index)
                    .iter()
                    .map(|source| Arc::downgrade(&pins[source.to_native() as usize]))
                    .collect(),
            );
        }

        // Pass 3: build nodes over their contiguous pin ranges.
        let mut nodes = AHashMap::with_capacity(plan.nodes.len());
        for index in 0..plan.nodes.len() as u32 {
            let entry = &plan.nodes[index as usize];
            let first = entry.first_pin.to_native() as usize;
            let end = first + entry.pin_count.to_native() as usize;

            let mut node_pins = AHashMap::with_capacity(end - first);
            let mut name_cache: AHashMap<String, Vec<Arc<InternalPin>>> = AHashMap::new();
            for pin in &pins[first..end] {
                node_pins.insert(pin.id.clone(), pin.clone());
                name_cache
                    .entry(pin.name.clone())
                    .or_default()
                    .push(pin.clone());
            }

            let node = self.node(index).ok_or(HydrateError::InvalidEnum {
                kind: "node index",
                value: 0,
            })?;
            let logic = self
                .logic(index)
                .expect("hydration resolved every node")
                .clone();
            let meta = &self.meta[index as usize];

            // Shares the definition rather than cloning it: this is per run, and a deep
            // clone here would reintroduce exactly the cost the plan removes.
            let internal = Arc::new(InternalNode::from_shared(
                node,
                NodeMeta {
                    id: meta.id.to_string(),
                    name: meta.name.to_string(),
                    is_pure: meta.is_pure,
                },
                node_pins.clone(),
                logic,
                name_cache,
            ));
            for pin in node_pins.values() {
                pin.init_node(Arc::downgrade(&internal));
            }

            nodes.insert(self.meta[index as usize].id.to_string(), internal);
        }

        let pin_map = pins
            .into_iter()
            .map(|pin| (pin.id.clone(), pin))
            .collect::<AHashMap<_, _>>();

        Ok(RuntimeGraph {
            nodes,
            pins: pin_map,
        })
    }
}

/// The mutable-per-run graph, in the shape the existing engine consumes.
pub struct RuntimeGraph {
    pub nodes: AHashMap<String, Arc<InternalNode>>,
    pub pins: AHashMap<String, Arc<InternalPin>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::board::{
        Board,
        compile::{CompileStamps, compile_board},
    };
    use crate::flow::node::NodeLogic;
    use flow_like_storage::Path;
    use flow_like_types::{async_trait, tokio};

    const STAMPS: CompileStamps = CompileStamps {
        catalog_signature: 1,
        wasm_signature: 2,
    };

    /// Minimal logic so the registry can resolve the fixture's node types.
    #[derive(Default)]
    struct Stub(&'static str);

    #[async_trait]
    impl NodeLogic for Stub {
        fn get_node(&self) -> Node {
            Node::new(self.0, self.0, "", "Test")
        }
        async fn run(
            &self,
            _context: &mut crate::flow::execution::context::ExecutionContext,
        ) -> flow_like_types::Result<()> {
            Ok(())
        }
    }

    fn registry() -> FlowNodeRegistryInner {
        let mut registry = FlowNodeRegistryInner::new(2);
        for name in ["events_simple", "log", "math_add"] {
            let logic: Arc<dyn NodeLogic> = Arc::new(Stub(name));
            registry.insert(logic.get_node(), logic);
        }
        registry
    }

    fn board() -> Board {
        let mut board = Board::new_detached(Some("board-h".into()), Path::from("apps/app-h"));

        let mut source = Node::new("events_simple", "Start", "entry", "Test");
        source.id = "node-src".into();
        let exec = {
            let pin = source.add_output_pin("exec_out", "Out", "", VariableType::Execution);
            pin.id = "pin-exec".into();
            "pin-exec".to_string()
        };
        source.pins = source
            .pins
            .drain()
            .map(|(_, pin)| (pin.id.clone(), pin))
            .collect();

        let mut sink = Node::new("log", "Log It", "writes a log line", "Test");
        sink.id = "node-sink".into();
        {
            let pin = sink.add_input_pin("exec_in", "In", "", VariableType::Execution);
            pin.id = "pin-exec-in".into();
        }
        {
            let pin = sink.add_input_pin("value", "Value", "the value", VariableType::String);
            pin.id = "pin-value".into();
            pin.set_default_value(Some(flow_like_types::json::json!("hello")));
        }
        sink.pins = sink
            .pins
            .drain()
            .map(|(_, pin)| (pin.id.clone(), pin))
            .collect();

        source
            .pins
            .get_mut(&exec)
            .unwrap()
            .connected_to
            .insert("pin-exec-in".into());
        sink.pins
            .get_mut("pin-exec-in")
            .unwrap()
            .depends_on
            .insert(exec);

        board.nodes.insert(source.id.clone(), source);
        board.nodes.insert(sink.id.clone(), sink);
        board
    }

    fn graph() -> CompiledGraph {
        let container = compile_board(&board(), STAMPS).unwrap().to_container().unwrap();
        CompiledGraph::hydrate(PlanBuffer::new(container).unwrap(), &registry()).unwrap()
    }

    #[test]
    fn hydration_resolves_logic_and_meta_for_every_node() {
        let graph = graph();
        assert_eq!(graph.node_count(), 2);

        let sink = graph.node_by_id("node-sink").expect("node present");
        let meta = graph.meta(sink).unwrap();
        assert_eq!(&*meta.id, "node-sink");
        assert_eq!(&*meta.name, "log");
        assert!(graph.logic(sink).is_some());
    }

    #[test]
    fn purity_is_carried_through_hydration() {
        let graph = graph();
        let src = graph.node_by_id("node-src").unwrap();
        let sink = graph.node_by_id("node-sink").unwrap();
        // Both fixture nodes carry execution pins, so neither is pure.
        assert!(!graph.meta(src).unwrap().is_pure);
        assert!(!graph.meta(sink).unwrap().is_pure);
    }

    #[test]
    fn defaults_are_parsed_once_and_shared() {
        let graph = graph();
        let plan = graph.plan();
        let value_pin = (0..plan.pins.len() as u32)
            .find(|index| plan.symbol(plan.pins[*index as usize].name.to_native()) == "value")
            .unwrap();

        let first = graph.pin_default(value_pin).expect("default present");
        let second = graph.pin_default(value_pin).unwrap();
        assert_eq!(*first, flow_like_types::json::json!("hello"));
        // The same parsed value is handed out, not re-parsed per call.
        assert!(Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn pins_without_defaults_report_none() {
        let graph = graph();
        let plan = graph.plan();
        let exec_pin = (0..plan.pins.len() as u32)
            .find(|index| plan.symbol(plan.pins[*index as usize].name.to_native()) == "exec_out")
            .unwrap();
        assert!(graph.pin_default(exec_pin).is_none());
    }

    #[tokio::test]
    async fn materialized_node_matches_the_source_board() {
        let graph = graph();
        let index = graph.node_by_id("node-sink").unwrap();
        let node_handle = graph.node(index).expect("node materializes");
        let node = node_handle.lock().await;

        assert_eq!(node.id, "node-sink");
        assert_eq!(node.name, "log");
        assert_eq!(node.friendly_name, "Log It");
        assert_eq!(node.description, "writes a log line");
        assert_eq!(node.pins.len(), 2);

        let value = node.pins.values().find(|pin| pin.name == "value").unwrap();
        assert_eq!(value.friendly_name, "Value");
        assert_eq!(value.description, "the value");
        assert_eq!(value.data_type, VariableType::String);
        assert_eq!(value.pin_type, PinType::Input);
        assert!(value.default_value.is_some());
    }

    /// Reconstruction must be cached — the whole point is to avoid per-execution rebuilds.
    #[test]
    fn materialized_nodes_are_cached() {
        let graph = graph();
        let index = graph.node_by_id("node-src").unwrap();
        assert!(Arc::ptr_eq(
            &graph.node(index).unwrap(),
            &graph.node(index).unwrap()
        ));
    }

    #[tokio::test]
    async fn reconstructed_edges_point_at_real_pin_ids() {
        let graph = graph();
        let source_handle = graph.node(graph.node_by_id("node-src").unwrap()).unwrap();
        let source = source_handle.lock().await;
        let exec = source.pins.values().find(|p| p.name == "exec_out").unwrap();
        assert!(exec.connected_to.contains("pin-exec-in"));
        drop(source);

        let sink_handle = graph.node(graph.node_by_id("node-sink").unwrap()).unwrap();
        let sink = sink_handle.lock().await;
        let exec_in = sink.pins.values().find(|p| p.name == "exec_in").unwrap();
        assert!(exec_in.depends_on.contains("pin-exec"));
    }

    #[test]
    fn unknown_node_types_fail_hydration_loudly() {
        let container = compile_board(&board(), STAMPS).unwrap().to_container().unwrap();
        let empty = FlowNodeRegistryInner::new(0);
        let error = CompiledGraph::hydrate(PlanBuffer::new(container).unwrap(), &empty)
            .expect_err("an unresolvable node type must not hydrate");
        assert!(matches!(error, HydrateError::UnknownNodeType { .. }));
    }

    #[test]
    fn stamps_survive_hydration() {
        let graph = graph();
        assert_eq!(graph.stamps().catalog_signature, 1);
        assert_eq!(graph.stamps().wasm_signature, 2);
    }

    /// The runtime graph the compiled path produces must describe exactly the same wiring
    /// as the board it was compiled from — that equivalence is what lets the old builder
    /// eventually be deleted.
    #[test]
    fn runtime_graph_matches_the_source_board() {
        let board = board();
        let graph = graph();
        let runtime = graph.build_runtime_graph().unwrap();

        assert_eq!(runtime.nodes.len(), board.nodes.len());
        let board_pins: usize = board.nodes.values().map(|node| node.pins.len()).sum();
        assert_eq!(runtime.pins.len(), board_pins);

        for (node_id, node) in &board.nodes {
            let internal = runtime
                .nodes
                .get(node_id)
                .unwrap_or_else(|| panic!("{node_id} missing from runtime graph"));
            assert_eq!(internal.node_id(), node_id);
            assert_eq!(internal.node_name(), node.name);
            assert_eq!(internal.pins.len(), node.pins.len());

            for pin in node.pins.values() {
                let internal_pin = runtime
                    .pins
                    .get(&pin.id)
                    .unwrap_or_else(|| panic!("pin {} missing", pin.id));
                assert_eq!(internal_pin.name, pin.name);
                assert_eq!(internal_pin.pin_type, pin.pin_type);
                assert_eq!(internal_pin.data_type, pin.data_type);
                assert_eq!(internal_pin.index, pin.index);
                assert_eq!(internal_pin.has_default, pin.default_value.is_some());
            }
        }
    }

    #[test]
    fn runtime_graph_wires_connections_and_dependencies() {
        let graph = graph();
        let runtime = graph.build_runtime_graph().unwrap();

        let exec_out = runtime.pins.get("pin-exec").unwrap();
        let connected = exec_out.connected_to();
        assert_eq!(connected.len(), 1);
        assert_eq!(
            connected[0].upgrade().unwrap().id,
            "pin-exec-in",
            "exec output must reach the sink's exec input"
        );

        let exec_in = runtime.pins.get("pin-exec-in").unwrap();
        let deps = exec_in.depends_on();
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].upgrade().unwrap().id, "pin-exec");
    }

    /// Pins must point back at their owning node, which the scheduler relies on when it
    /// walks an edge to decide what to run next.
    #[test]
    fn runtime_pins_know_their_owning_node() {
        let graph = graph();
        let runtime = graph.build_runtime_graph().unwrap();

        let pin = runtime.pins.get("pin-value").unwrap();
        let owner = pin.node().expect("pin has an owner").upgrade().unwrap();
        assert_eq!(owner.node_id(), "node-sink");
    }

    #[test]
    fn runtime_graph_carries_pre_parsed_defaults() {
        let graph = graph();
        let runtime = graph.build_runtime_graph().unwrap();

        let pin = runtime.pins.get("pin-value").unwrap();
        assert!(pin.has_default);
        assert_eq!(
            pin.default_value.as_ref().unwrap(),
            &flow_like_types::json::json!("hello")
        );
    }

    /// Each run needs its own value cells; sharing them across runs would leak state.
    /// Node definitions are read-only during a run, so every run must share one `Arc`
    /// rather than deep-cloning the node (pin map and dependency sets included) per run.
    #[test]
    fn runtime_graphs_share_node_definitions() {
        let graph = graph();
        let first = graph.build_runtime_graph().unwrap();
        let second = graph.build_runtime_graph().unwrap();

        assert!(Arc::ptr_eq(
            &first.nodes.get("node-sink").unwrap().node,
            &second.nodes.get("node-sink").unwrap().node
        ));
    }

    #[test]
    fn each_runtime_graph_gets_fresh_pins() {
        let graph = graph();
        let first = graph.build_runtime_graph().unwrap();
        let second = graph.build_runtime_graph().unwrap();

        assert!(!Arc::ptr_eq(
            first.pins.get("pin-value").unwrap(),
            second.pins.get("pin-value").unwrap()
        ));
    }
}
