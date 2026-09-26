use std::{
    collections::{BTreeMap, HashMap, HashSet},
    future::Future,
    io::{self, Write},
    panic::AssertUnwindSafe,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use flow_like_runtime::{
    flow::{
        board::{Board, LayerType},
        compiled::template::CompiledRunTemplate,
        execution::{
            ExecutionEnvironment, InternalRun, LogLevel, RunPayload, RunStatus,
            context::ExecutionContext,
        },
        node::{Node, NodeLogic},
        pin::PinType,
        variable::VariableType,
    },
    profile::Profile,
    state::{FlowLikeConfig, FlowLikeState},
    utils::http::HTTPClient,
};
use flow_like_storage::{Path, files::store::FlowLikeStore, object_store::memory::InMemory};
use flow_like_types::{
    Value, async_trait,
    channel::{Channel, ChannelClientDescriptor, ChannelHandle, ChannelOutcome, ChannelTicket},
    intercom::InterComCallback,
    tokio,
    tokio_util::sync::CancellationToken,
};
use futures::FutureExt;
use serde::{Deserialize, Serialize};

const MAX_BOARD_BYTES: usize = 512 * 1024;
const MAX_VALUE_BYTES: usize = 16 * 1024;
const MAX_NODES: usize = 64;
const MAX_PINS: usize = 512;
const MAX_EDGES: usize = 2048;
const MAX_INVOCATIONS: u64 = 256;
const MAX_OUTPUTS: usize = 16;
const MAX_ERRORS: usize = 16;
const DEADLINE: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DraftTestStatus {
    Success,
    Failed,
    Timeout,
    LimitExceeded,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftTestResult {
    pub status: DraftTestStatus,
    /// Present only when exactly one result was emitted by the runtime.
    pub output: Option<Value>,
    pub outputs: Vec<Value>,
    pub nodes_executed: u64,
    /// Local Function layer IDs mapped to actual native call dispatch counts.
    #[serde(default)]
    pub helper_calls: BTreeMap<String, u64>,
    pub errors: Vec<String>,
}

#[derive(Default)]
struct Evidence {
    invocations: AtomicU64,
    limit_exceeded: AtomicBool,
    errors: Mutex<Vec<String>>,
    outputs: Mutex<Vec<Value>>,
    helper_calls: Mutex<BTreeMap<String, u64>>,
}

impl Evidence {
    fn error(&self, message: impl Into<String>) {
        let mut errors = self.errors.lock().unwrap_or_else(|e| e.into_inner());
        if errors.len() < MAX_ERRORS {
            let message = message.into();
            errors.push(message.chars().take(1024).collect());
        }
    }

    fn limit(&self, message: &str) -> flow_like_types::Error {
        self.limit_exceeded.store(true, Ordering::Relaxed);
        self.error(message);
        flow_like_types::anyhow!("{message}")
    }

    fn result(&self, fallback: DraftTestStatus) -> DraftTestResult {
        let outputs = self
            .outputs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let errors = self
            .errors
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let status = if self.limit_exceeded.load(Ordering::Relaxed) {
            DraftTestStatus::LimitExceeded
        } else if fallback == DraftTestStatus::Timeout {
            fallback
        } else if !errors.is_empty() {
            DraftTestStatus::Failed
        } else {
            fallback
        };
        DraftTestResult {
            status,
            output: (outputs.len() == 1).then(|| outputs[0].clone()),
            outputs,
            nodes_executed: self.invocations.load(Ordering::Relaxed),
            helper_calls: self
                .helper_calls
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone(),
            errors,
        }
    }
}

struct SizeLimit(usize);
impl Write for SizeLimit {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.0 {
            return Err(io::Error::other("draft test size limit exceeded"));
        }
        self.0 -= bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn check_size(value: &impl Serialize, max: usize) -> Result<(), String> {
    serde_json::to_writer(SizeLimit(max), value).map_err(|e| e.to_string())
}

fn check_value(value: &Value) -> Result<(), String> {
    let mut stack = vec![(value, 0usize)];
    let mut count = 0;
    while let Some((value, depth)) = stack.pop() {
        count += 1;
        if depth > 32 || count > 4096 {
            return Err("draft test value nesting or item limit exceeded".into());
        }
        match value {
            Value::Array(values) => {
                if count + stack.len() + values.len() > 4096 {
                    return Err("draft test value item limit exceeded".into());
                }
                stack.extend(values.iter().map(|v| (v, depth + 1)));
            }
            Value::Object(values) => {
                if count + stack.len() + values.len() > 4096 {
                    return Err("draft test value item limit exceeded".into());
                }
                stack.extend(values.values().map(|v| (v, depth + 1)));
            }
            _ => {}
        }
    }
    check_size(value, MAX_VALUE_BYTES)
}

// Construct these types directly. A host registry can override a builtin by name. string_format
// remains excluded: repeated placeholder substitution can amplify values before output checks.
fn native_nodes() -> Vec<Arc<dyn NodeLogic>> {
    use flow_like_catalog_data::events::{
        generic_event::{GenericEventNode, push_generic_result::ReturnGenericResultNode},
        simple_event::SimpleEventNode,
    };
    use flow_like_catalog_std::{
        control::{branch_node::BranchNode, call_function::CallFunctionNode},
        structs::{
            fields::{get_field::GetStructFieldNode, set_field::SetStructFieldNode},
            make::MakeStructNode,
        },
        testing::flow_assert::FlowAssertNode,
        utils::{
            array::{
                construct::ConstructArrayNode, get::GetArrayElementNode, len::ArrayLengthNode,
                push::PushArrayNode,
            },
            bool::{
                and::BoolAnd, equal::BoolEqual, not::BoolNot, or::BoolOr, unequal::BoolUnequalNode,
                xor::BoolXor,
            },
            float::{
                add::AddFloatNode, divide::DivideFloatNode, equal::EqualFloatNode,
                gt::GreaterThanFloatNode, gte::GreaterThanOrEqualFloatNode, lt::LessThanFloatNode,
                lte::LessThanOrEqualFloatNode, multiply::MultiplyFloatNode, pow::PowerFloatNode,
                subtract::SubtractFloatNode, unequal::UnequalFloatNode,
            },
            int::{
                add::AddIntegerNode, divide::DivideIntegerNode, equal::EqualIntegerNode,
                gt::GreaterThanIntegerNode, gte::GreaterThanOrEqualIntegerNode,
                lt::LessThanIntegerNode, lte::LessThanOrEqualIntegerNode,
                modulo::ModuloIntegerNode, multiply::MultiplyIntegerNode, pow::PowerIntegerNode,
                subtract::SubtractIntegerNode, unequal::UnequalIntegerNode,
            },
            string::{
                chars::StringConcatNode, contains::StringContainsNode, equal::EqualStringNode,
                length::StringLengthNode, to_lowercase::StringToLowerNode,
                to_uppercase::StringToUpperNode, trim::StringTrimNode, unequal::UnEqualStringNode,
            },
            types::select::SelectNode,
        },
    };
    vec![
        Arc::new(GenericEventNode::new()),
        Arc::new(SimpleEventNode::new()),
        Arc::new(ReturnGenericResultNode::new()),
        Arc::new(BranchNode::new()),
        Arc::new(CallFunctionNode::new()),
        Arc::new(FlowAssertNode::new()),
        Arc::new(GetStructFieldNode::new()),
        Arc::new(SetStructFieldNode::new()),
        Arc::new(MakeStructNode::new()),
        Arc::new(GetArrayElementNode::new()),
        Arc::new(ArrayLengthNode::new()),
        Arc::new(ConstructArrayNode::new()),
        Arc::new(PushArrayNode::new()),
        Arc::new(BoolAnd::new()),
        Arc::new(BoolOr::new()),
        Arc::new(BoolNot::new()),
        Arc::new(BoolEqual::new()),
        Arc::new(BoolUnequalNode::new()),
        Arc::new(BoolXor::new()),
        Arc::new(SelectNode::new()),
        Arc::new(AddFloatNode::new()),
        Arc::new(SubtractFloatNode::new()),
        Arc::new(MultiplyFloatNode::new()),
        Arc::new(DivideFloatNode::new()),
        Arc::new(PowerFloatNode::new()),
        Arc::new(EqualFloatNode::new()),
        Arc::new(UnequalFloatNode::new()),
        Arc::new(GreaterThanFloatNode::new()),
        Arc::new(GreaterThanOrEqualFloatNode::new()),
        Arc::new(LessThanFloatNode::new()),
        Arc::new(LessThanOrEqualFloatNode::new()),
        Arc::new(AddIntegerNode::new()),
        Arc::new(SubtractIntegerNode::new()),
        Arc::new(MultiplyIntegerNode::new()),
        Arc::new(DivideIntegerNode::new()),
        Arc::new(ModuloIntegerNode::new()),
        Arc::new(PowerIntegerNode::new()),
        Arc::new(EqualIntegerNode::new()),
        Arc::new(UnequalIntegerNode::new()),
        Arc::new(GreaterThanIntegerNode::new()),
        Arc::new(GreaterThanOrEqualIntegerNode::new()),
        Arc::new(LessThanIntegerNode::new()),
        Arc::new(LessThanOrEqualIntegerNode::new()),
        Arc::new(StringContainsNode::new()),
        Arc::new(EqualStringNode::new()),
        Arc::new(UnEqualStringNode::new()),
        Arc::new(StringConcatNode::new()),
        Arc::new(StringLengthNode::new()),
        Arc::new(StringTrimNode::new()),
        Arc::new(StringToLowerNode::new()),
        Arc::new(StringToUpperNode::new()),
    ]
}

/// The concrete catalog accepted by the isolated runner, also usable for draft preparation.
pub fn supported_node_names() -> Vec<String> {
    let mut names: Vec<_> = native_nodes().iter().map(|n| n.get_node().name).collect();
    names.sort();
    names
}

/// Check whether the current board fits the isolated JSON runner's structural and resource
/// limits. Empty boards are eligible. This read-only check does not execute or modify the board,
/// select an entry, or establish that its outputs satisfy the requested behavior.
pub fn validate_isolated_json_board(board: &Board) -> Result<(), String> {
    validate_board(board)
}

fn validate_board(board: &Board) -> Result<(), String> {
    check_size(board, MAX_BOARD_BYTES)?;
    let allowed: HashSet<_> = supported_node_names().into_iter().collect();
    let nodes: Vec<_> = board
        .nodes
        .values()
        .chain(board.layers.values().flat_map(|l| l.nodes.values()))
        .collect();
    if nodes.len() > MAX_NODES || board.layers.len() > MAX_NODES {
        return Err(format!(
            "draft tests allow at most {MAX_NODES} nodes and layers"
        ));
    }
    let mut ids = HashSet::new();
    for node in &nodes {
        if !ids.insert(&node.id) {
            return Err("duplicate draft node id".into());
        }
        if node.wasm.is_some() || !allowed.contains(&node.name) {
            return Err(format!(
                "node '{}' ({}) is unsupported in isolated draft tests",
                node.name, node.id
            ));
        }
        if node.fn_refs.as_ref().is_some_and(|r| !r.fn_refs.is_empty()) {
            return Err("function references are unsupported in isolated draft tests".into());
        }
    }
    for layer in board.layers.values() {
        if matches!(layer.r#type, LayerType::Macro) || layer.cache.is_some() {
            return Err("macros and layer caches are unsupported in isolated draft tests".into());
        }
    }
    if !board.variables.is_empty() || board.layers.values().any(|l| !l.variables.is_empty()) {
        return Err(
            "variables are unsupported in isolated draft tests; supply a test payload".into(),
        );
    }
    let pins: Vec<_> = nodes
        .iter()
        .flat_map(|n| n.pins.values())
        .chain(board.layers.values().flat_map(|l| l.pins.values()))
        .collect();
    if pins.len() > MAX_PINS {
        return Err(format!("draft tests allow at most {MAX_PINS} pins"));
    }
    let mut ids = HashSet::new();
    let mut edges = 0usize;
    for pin in pins {
        if !ids.insert(&pin.id) {
            return Err("duplicate draft pin id".into());
        }
        if !matches!(
            pin.data_type,
            VariableType::Execution
                | VariableType::Generic
                | VariableType::Struct
                | VariableType::String
                | VariableType::Integer
                | VariableType::Float
                | VariableType::Boolean
        ) {
            return Err("draft tests support only JSON primitive, struct and array pins".into());
        }
        edges = edges
            .saturating_add(pin.depends_on.len())
            .saturating_add(pin.connected_to.len());
        if edges > MAX_EDGES {
            return Err("draft test edge limit exceeded".into());
        }
        if let Some(bytes) = &pin.default_value {
            if bytes.len() > MAX_VALUE_BYTES {
                return Err("draft pin default exceeds size limit".into());
            }
            let value: Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
            check_value(&value)?;
        }
    }
    validate_function_boundaries(board)?;
    Ok(())
}

fn owning_function<'a>(
    board: &'a Board,
    layer_id: Option<&'a str>,
) -> Result<Option<&'a str>, String> {
    let mut current = layer_id;
    let mut owner = None;
    let mut seen = HashSet::new();
    while let Some(id) = current {
        if !seen.insert(id) {
            return Err("cyclic layer containment is unsupported in isolated draft tests".into());
        }
        let layer = board
            .layers
            .get(id)
            .ok_or_else(|| "draft references a missing layer".to_string())?;
        if owner.is_none() && matches!(layer.r#type, LayerType::Function) {
            owner = Some(id);
        }
        current = layer.parent_id.as_deref();
    }
    Ok(owner)
}

// Calls may only enter local functions through their mirrored call pins. Direct wires across
// function scopes could otherwise create recursion that is invisible in the declared call graph.
fn validate_function_boundaries(board: &Board) -> Result<(), String> {
    let mut scopes = HashMap::new();
    let mut nodes = Vec::new();
    for (id, node) in &board.nodes {
        if id != &node.id {
            return Err("draft node map key does not match its id".into());
        }
        nodes.push((node, owning_function(board, node.layer.as_deref())?));
    }
    let mut calls: HashMap<&str, HashSet<&str>> = HashMap::new();
    for (id, layer) in &board.layers {
        if id != &layer.id {
            return Err("draft layer map key does not match its id".into());
        }
        let scope = owning_function(board, Some(id))?;
        if matches!(layer.r#type, LayerType::Function) {
            calls.insert(id, HashSet::new());
            if layer
                .pins
                .values()
                .any(|pin| pin.name == "function_layer_id")
            {
                return Err("function_layer_id is reserved for static local helper calls".into());
            }
        }
        for pin in layer.pins.values() {
            scopes.insert(pin.id.as_str(), scope);
        }
        for (node_id, node) in &layer.nodes {
            if node_id != &node.id {
                return Err("draft layer node map key does not match its id".into());
            }
            nodes.push((node, scope));
        }
    }
    for (node, scope) in &nodes {
        for pin in node.pins.values() {
            scopes.insert(pin.id.as_str(), *scope);
        }
    }
    for (node, scope) in &nodes {
        if node.name != "control_call_function" {
            continue;
        }
        let targets: Vec<_> = node
            .pins
            .values()
            .filter(|pin| pin.name == "function_layer_id")
            .collect();
        let target = match targets.as_slice() {
            [pin]
                if pin.pin_type == PinType::Input
                    && pin.data_type == VariableType::String
                    && pin.depends_on.is_empty() =>
            {
                pin
            }
            _ => {
                return Err(
                    "helper calls require one statically bound function_layer_id input".into(),
                );
            }
        };
        let target: String = target
            .default_value
            .as_deref()
            .and_then(|value| serde_json::from_slice(value).ok())
            .ok_or_else(|| "helper calls require a literal local function_layer_id".to_string())?;
        let target_layer = board
            .layers
            .get(&target)
            .filter(|layer| matches!(layer.r#type, LayerType::Function))
            .ok_or_else(|| {
                "helper call target must be a Function layer on this board".to_string()
            })?;
        if let Some(scope) = scope {
            calls
                .get_mut(scope)
                .ok_or_else(|| "helper call has an invalid function owner".to_string())?
                .insert(target_layer.id.as_str());
        }
    }
    for pin in nodes
        .iter()
        .flat_map(|(node, _)| node.pins.values())
        .chain(board.layers.values().flat_map(|layer| layer.pins.values()))
    {
        let scope = scopes[pin.id.as_str()];
        for target in pin.depends_on.iter().chain(&pin.connected_to) {
            let target_scope = scopes
                .get(target.as_str())
                .ok_or_else(|| "draft wire references a missing pin".to_string())?;
            if target_scope != &scope {
                return Err("direct wires across helper function boundaries are unsupported; call the helper".into());
            }
        }
    }
    let mut indegrees: HashMap<&str, usize> = calls.keys().map(|id| (*id, 0)).collect();
    for targets in calls.values() {
        for target in targets {
            *indegrees
                .get_mut(target)
                .expect("validated function target") += 1;
        }
    }
    let mut ready: Vec<_> = indegrees
        .iter()
        .filter_map(|(id, degree)| (*degree == 0).then_some(*id))
        .collect();
    let mut visited = 0;
    while let Some(id) = ready.pop() {
        visited += 1;
        for target in &calls[id] {
            let degree = indegrees
                .get_mut(target)
                .expect("validated function target");
            *degree -= 1;
            if *degree == 0 {
                ready.push(*target);
            }
        }
    }
    if visited != calls.len() {
        return Err("recursive helper calls are unsupported in isolated draft tests".into());
    }
    Ok(())
}

struct BoundedNode {
    inner: Arc<dyn NodeLogic>,
    evidence: Arc<Evidence>,
}

#[async_trait]
impl NodeLogic for BoundedNode {
    fn get_node(&self) -> Node {
        self.inner.get_node()
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        self.inner.on_update(node, board).await;
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        if self.evidence.limit_exceeded.load(Ordering::Relaxed) {
            return Err(flow_like_types::anyhow!(
                "draft test stopped after resource limit"
            ));
        }
        // Both runtime dispatch paths, including dependency pulls, call this wrapper.
        if self
            .evidence
            .invocations
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                (n < MAX_INVOCATIONS).then_some(n + 1)
            })
            .is_err()
        {
            return Err(self
                .evidence
                .limit("draft test node invocation limit exceeded"));
        }
        tokio::task::yield_now().await;
        if context.is_cancelled() {
            self.evidence.error("draft test cancelled");
            return Err(flow_like_types::anyhow!("draft test cancelled"));
        }
        for pin in context.node.pins.iter() {
            if let Some(value) = context
                .context_pin_overrides
                .as_ref()
                .and_then(|values| values.get(pin.id.as_ref()))
            {
                check_value(value).map_err(|_| {
                    self.evidence
                        .limit("draft test scoped pin value limit exceeded")
                })?;
            }
            if let Some(value) = pin.value.read().as_ref() {
                check_value(value)
                    .map_err(|_| self.evidence.limit("draft test pin value limit exceeded"))?;
            }
        }
        // Guard native arithmetic before debug/release overflow or nonfinite-to-JSON-null
        // coercion can change its meaning. The native implementation still produces the output.
        let input_check: flow_like_types::Result<()> = async {
            let node_type = context.node.node_name();
            if matches!(
                node_type,
                "int_add"
                    | "int_subtract"
                    | "int_multiply"
                    | "int_divide"
                    | "int_modulo"
                    | "int_power"
            ) {
                let (a_pin, b_pin) = if node_type == "int_power" {
                    ("base", "exponent")
                } else {
                    ("integer1", "integer2")
                };
                let a: i64 = context.evaluate_pin(a_pin).await?;
                let b: i64 = context.evaluate_pin(b_pin).await?;
                if matches!(node_type, "int_divide" | "int_modulo") && b == 0 {
                    return Err(flow_like_types::anyhow!(
                        "integer division or remainder by zero"
                    ));
                }
                let checked = match node_type {
                    "int_add" => a.checked_add(b).map(|_| ()),
                    "int_subtract" => a.checked_sub(b).map(|_| ()),
                    "int_multiply" => a.checked_mul(b).map(|_| ()),
                    "int_modulo" => a.checked_rem(b).map(|_| ()),
                    "int_power" => {
                        let exponent = u32::try_from(b).map_err(|_| {
                            flow_like_types::anyhow!(
                                "integer exponent must be between zero and u32::MAX"
                            )
                        })?;
                        a.checked_pow(exponent).map(|_| ())
                    }
                    // Integer `/` returns Float; its nonzero i64 operands cannot overflow f64.
                    "int_divide" => Some(()),
                    _ => unreachable!(),
                };
                if checked.is_none() {
                    return Err(flow_like_types::anyhow!("integer arithmetic overflow"));
                }
            }
            if matches!(
                node_type,
                "float_add" | "float_subtract" | "float_multiply" | "float_divide" | "float_power"
            ) {
                let (a_pin, b_pin) = match node_type {
                    "float_divide" => ("dividend", "divisor"),
                    "float_power" => ("base", "exponent"),
                    _ => ("float1", "float2"),
                };
                let a: f64 = context.evaluate_pin(a_pin).await?;
                let b: f64 = context.evaluate_pin(b_pin).await?;
                if node_type == "float_divide" && b == 0.0 {
                    return Err(flow_like_types::anyhow!("float division by zero"));
                }
                let result = match node_type {
                    "float_add" => a + b,
                    "float_subtract" => a - b,
                    "float_multiply" => a * b,
                    "float_divide" => a / b,
                    "float_power" => a.powf(b),
                    _ => unreachable!(),
                };
                if !a.is_finite() || !b.is_finite() || !result.is_finite() {
                    return Err(flow_like_types::anyhow!(
                        "nonfinite float arithmetic result"
                    ));
                }
            }
            if node_type == "string_concat" {
                let mut bytes = 0usize;
                for pin in context.get_pins_by_name("string").await? {
                    let part: String = context.evaluate_pin_ref(pin).await?;
                    bytes = bytes.saturating_add(part.len());
                    if bytes > MAX_VALUE_BYTES {
                        return Err(self
                            .evidence
                            .limit("draft concatenated string exceeds size limit"));
                    }
                }
            }
            Ok(())
        }
        .await;
        if let Err(error) = input_check {
            self.evidence.error(error.to_string());
            return Err(error);
        }
        if context.node.node_name() == "struct_set" {
            let field: String = context.evaluate_pin("field").await.map_err(|e| {
                self.evidence.error(e.to_string());
                e
            })?;
            // The native setter can create an object for every missing path segment.
            if field.chars().filter(|c| matches!(c, '.' | '[')).count() >= 32 {
                return Err(self
                    .evidence
                    .limit("draft struct field path nesting limit exceeded"));
            }
        }
        let helper_target: Option<String> = if context.node.node_name() == "control_call_function" {
            Some(
                context
                    .evaluate_pin("function_layer_id")
                    .await
                    .map_err(|e| {
                        self.evidence.error(e.to_string());
                        e
                    })?,
            )
        } else {
            None
        };
        // Another branch can hit a limit while this invocation yields or reads its inputs.
        if self.evidence.limit_exceeded.load(Ordering::Relaxed) {
            return Err(flow_like_types::anyhow!(
                "draft test stopped after resource limit"
            ));
        }
        if let Some(target) = helper_target {
            *self
                .evidence
                .helper_calls
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .entry(target)
                .or_default() += 1;
        }
        let result = AssertUnwindSafe(self.inner.run(context))
            .catch_unwind()
            .await;
        match result {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                self.evidence.error(error.to_string());
                return Err(error);
            }
            Err(_) => {
                self.evidence.error("native node panicked");
                return Err(flow_like_types::anyhow!("native node panicked"));
            }
        }
        for pin in context.node.pins.iter() {
            if pin.pin_type == PinType::Output
                && let Some(value) = context
                    .context_pin_overrides
                    .as_ref()
                    .and_then(|values| values.get(pin.id.as_ref()))
            {
                check_value(value).map_err(|_| {
                    self.evidence
                        .limit("draft test scoped output pin limit exceeded")
                })?;
            }
            if pin.pin_type == PinType::Output
                && let Some(value) = pin.value.read().as_ref()
            {
                check_value(value)
                    .map_err(|_| self.evidence.limit("draft test output pin limit exceeded"))?;
            }
        }
        Ok(())
    }
}

// No global in-process registration, external transport or inbound messages.
struct ClosedChannel;
#[async_trait]
impl Channel for ClosedChannel {
    fn channel_id(&self) -> &str {
        "isolated-draft-test"
    }
    fn handle(&self) -> ChannelHandle {
        ChannelHandle {
            channel_id: self.channel_id().into(),
            request_id: None,
            expires_at: 0,
            transport: ChannelClientDescriptor::InProcess {},
            fallback: None,
        }
    }
    async fn open(&self, _: Duration) -> flow_like_types::Result<ChannelTicket> {
        Err(flow_like_types::anyhow!("draft test channels are disabled"))
    }
    async fn wait(
        &self,
        _: &ChannelTicket,
        _: Option<CancellationToken>,
    ) -> flow_like_types::Result<ChannelOutcome> {
        Ok(ChannelOutcome::Closed)
    }
    async fn abandon(&self, _: &ChannelTicket) {}
    async fn drain_inbound(&self) -> Vec<Value> {
        vec![]
    }
    async fn is_cancelled(&self) -> bool {
        false
    }
    async fn close(&self) {}
}

pub async fn test_draft_board(
    board: Board,
    entry: &str,
    payload: Option<Value>,
) -> Result<DraftTestResult, String> {
    prepare_and_test_draft_board(board, entry, payload, |board, _| async { Ok(board) }).await
}

fn detach_board(board: &Board) -> Result<Board, String> {
    serde_json::from_slice(&serde_json::to_vec(board).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}

async fn prepare_isolated_board<F, Fut>(
    board: Board,
    evidence: Arc<Evidence>,
    prepare: F,
) -> Result<(Board, Arc<FlowLikeState>), String>
where
    F: FnOnce(Board, Arc<FlowLikeState>) -> Fut,
    Fut: Future<Output = Result<Board, String>>,
{
    validate_isolated_json_board(&board)?;
    // Drop parent pointers, cached implementations and host state before host preparation.
    let board = detach_board(&board)?;
    let store = FlowLikeStore::Memory(Arc::new(InMemory::new()));
    let mut config = FlowLikeConfig::with_default_store(store);
    config.stores.log_store = None;
    let mut state = FlowLikeState::new(config, HTTPClient::new_without_refetch());
    state.execution_environment = ExecutionEnvironment::Server;
    let state = Arc::new(state);
    state.node_registry.write().await.push_nodes(
        native_nodes()
            .into_iter()
            .map(|inner| {
                Arc::new(BoundedNode {
                    inner,
                    evidence: evidence.clone(),
                }) as Arc<dyn NodeLogic>
            })
            .collect(),
    );
    let board = prepare(board, state.clone()).await?;
    validate_isolated_json_board(&board)?;
    // Preparation must not retain host pointers in its returned board either.
    let mut board = detach_board(&board)?;
    board.board_dir = Path::from("isolated-draft-test");
    board.log_level = LogLevel::Error;
    Ok((board, state))
}

/// Materialize and validate a detached board using the test runner's restricted state.
/// The callback is trusted host code; this operation does not execute the resulting workflow.
pub async fn prepare_draft_board<F, Fut>(board: Board, prepare: F) -> Result<Board, String>
where
    F: FnOnce(Board, Arc<FlowLikeState>) -> Fut,
    Fut: Future<Output = Result<Board, String>>,
{
    let task = prepare_isolated_board(board, Arc::new(Evidence::default()), prepare);
    match tokio::time::timeout(DEADLINE, AssertUnwindSafe(task).catch_unwind()).await {
        Ok(Ok(result)) => result.map(|(board, _)| board),
        Ok(Err(_)) => Err("draft preparation panicked".into()),
        Err(_) => Err("draft preparation exceeded its three-second deadline".into()),
    }
}

/// Execute a detached draft with trusted native nodes and disposable memory stores.
/// The preparation callback is host code; it receives only the restricted state.
/// Unsupported nodes are rejected before preparation and again before compilation.
pub async fn prepare_and_test_draft_board<F, Fut>(
    board: Board,
    entry: &str,
    payload: Option<Value>,
    prepare: F,
) -> Result<DraftTestResult, String>
where
    F: FnOnce(Board, Arc<FlowLikeState>) -> Fut,
    Fut: Future<Output = Result<Board, String>>,
{
    if let Some(value) = &payload {
        check_value(value)?;
    }
    let evidence = Arc::new(Evidence::default());
    let cancellation = CancellationToken::new();
    let _cancel_on_drop = cancellation.clone().drop_guard();
    let task_evidence = evidence.clone();
    let task_cancel = cancellation.clone();
    let task = async move {
        let (board, state) = prepare_isolated_board(board, task_evidence.clone(), prepare).await?;
        let matches: Vec<_> = board
            .nodes
            .values()
            .chain(board.layers.values().flat_map(|l| l.nodes.values()))
            .filter(|n| n.id == entry || n.friendly_name == entry)
            .collect();
        if matches.len() != 1 {
            return Err("entry must identify exactly one node by id or friendly name".into());
        }
        let entry_node = matches[0];
        if !matches!(entry_node.name.as_str(), "events_generic" | "events_simple")
            || entry_node.start != Some(true)
        {
            return Err("entry must be a generic or simple event".into());
        }
        let entry_layer = board
            .layers
            .values()
            .find(|layer| layer.nodes.contains_key(&entry_node.id))
            .map(|layer| layer.id.as_str())
            .or(entry_node.layer.as_deref());
        if owning_function(&board, entry_layer)?.is_some() {
            return Err(
                "select a board event that calls the helper, not an event inside a function".into(),
            );
        }
        let payload = RunPayload {
            id: entry_node.id.clone(),
            payload,
            runtime_variables: None,
            filter_secrets: Some(true),
        };
        let callback_evidence = task_evidence.clone();
        let callback: InterComCallback = Some(Arc::new(move |event| {
            let evidence = callback_evidence.clone();
            Box::pin(async move {
                if event.event_type == "generic_result" {
                    if check_value(&event.payload).is_err() {
                        return Err(evidence.limit("draft result exceeds size limit"));
                    }
                    let mut outputs = evidence.outputs.lock().unwrap_or_else(|e| e.into_inner());
                    if outputs.len() >= MAX_OUTPUTS {
                        return Err(evidence.limit("draft result count limit exceeded"));
                    }
                    outputs.push(event.payload);
                }
                Ok(())
            })
        }));
        let registry = state.node_registry.read().await.node_registry.clone();
        let template = Arc::new(
            CompiledRunTemplate::from_board(Arc::new(board), &registry)
                .map_err(|e| e.to_string())?,
        );
        let mut run = InternalRun::from_template(
            "isolated-draft-test",
            template,
            None,
            &state,
            &Profile::default(),
            &payload,
            false,
            callback,
            None,
            None,
            HashMap::new(),
            None,
            Some(Arc::new(ClosedChannel)),
        )
        .await
        .map_err(|e| e.to_string())?;
        run.set_cancellation_token(task_cancel);
        run.execute(state).await;
        let status = run.get_status().await;
        {
            // The runtime can recover through an error branch and finish successfully.
            // A test still records the original failure, including dependency errors that
            // happen before a native implementation is called.
            let record = run.run.lock().await;
            for log in record
                .traces
                .iter()
                .flat_map(|trace| &trace.logs)
                .filter(|log| log.log_level >= LogLevel::Error)
                .take(MAX_ERRORS)
            {
                task_evidence.error(&log.message);
            }
            if record.highest_log_level >= LogLevel::Error {
                task_evidence.error("runtime recorded an error");
            }
        }
        if !matches!(status, RunStatus::Success) {
            task_evidence.error(format!("runtime finished with {status:?}"));
        }
        let count = task_evidence
            .outputs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .len();
        if count != 1 {
            task_evidence.error(format!(
                "expected exactly one generic result, runtime emitted {count}"
            ));
        }
        Ok(task_evidence.result(DraftTestStatus::Success))
    };
    match tokio::time::timeout(DEADLINE, AssertUnwindSafe(task).catch_unwind()).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => {
            evidence.error("draft runtime panicked");
            Ok(evidence.result(DraftTestStatus::Failed))
        }
        Err(_) => {
            cancellation.cancel();
            evidence.error("draft test exceeded its three-second deadline");
            Ok(evidence.result(DraftTestStatus::Timeout))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_runtime::flow::board::Layer;
    use serde_json::json;

    fn board() -> Board {
        Board::new_detached(Some("draft".into()), Path::from("unused"))
    }

    fn add(board: &mut Board, name: &str, id: &str) {
        let mut node = native_nodes()
            .into_iter()
            .map(|n| n.get_node())
            .find(|n| n.name == name)
            .unwrap();
        node.id = id.into();
        node.friendly_name = id.into();
        board.nodes.insert(id.into(), node);
    }

    fn wire(board: &mut Board, from: &str, output: &str, to: &str, input: &str) {
        let output_id = board.nodes[from]
            .pins
            .values()
            .find(|p| p.name == output)
            .unwrap()
            .id
            .clone();
        let input_id = board.nodes[to]
            .pins
            .values()
            .find(|p| p.name == input)
            .unwrap()
            .id
            .clone();
        board
            .nodes
            .get_mut(from)
            .unwrap()
            .pins
            .get_mut(&output_id)
            .unwrap()
            .connected_to
            .insert(input_id.clone());
        board
            .nodes
            .get_mut(to)
            .unwrap()
            .pins
            .get_mut(&input_id)
            .unwrap()
            .depends_on
            .insert(output_id);
    }

    fn literal(board: &mut Board, node: &str, pin: &str, value: Value) {
        board
            .nodes
            .get_mut(node)
            .unwrap()
            .pins
            .values_mut()
            .find(|p| p.name == pin)
            .unwrap()
            .set_default_value(Some(value));
    }

    fn return_board(value: Value) -> Board {
        let mut board = board();
        add(&mut board, "events_simple", "test");
        add(&mut board, "events_generic_return_result", "result");
        wire(&mut board, "test", "exec_out", "result", "exec_in");
        literal(&mut board, "result", "response", value);
        board
    }

    #[test]
    fn isolated_json_eligibility_accepts_empty_and_supported_boards_without_execution_or_mutation()
    {
        let empty = board();
        let empty_before = serde_json::to_value(&empty).unwrap();
        assert!(validate_isolated_json_board(&empty).is_ok());
        assert_eq!(serde_json::to_value(&empty).unwrap(), empty_before);

        let called = Arc::new(AtomicBool::new(false));
        let mut draft = return_board(json!({"label": "eligible"}));
        let host_logic: Arc<dyn NodeLogic> = Arc::new(HostOverride(called.clone()));
        draft.logic_nodes.insert("test".into(), host_logic.clone());
        let before = serde_json::to_value(&draft).unwrap();
        assert!(validate_isolated_json_board(&draft).is_ok());
        assert_eq!(serde_json::to_value(&draft).unwrap(), before);
        assert!(Arc::ptr_eq(&draft.logic_nodes["test"], &host_logic));
        assert!(!called.load(Ordering::SeqCst));
    }

    #[test]
    fn isolated_json_eligibility_rejects_unsupported_nodes_variables_and_caches() {
        use flow_like_runtime::flow::{pin::ValueType, variable::Variable};

        for nested in [false, true] {
            let mut draft = return_board(json!(1));
            let mut layer = Layer::new("helper".into(), "helper".into(), LayerType::Function);
            let unsupported = Node::new("http_request", "Request", "", "");
            if nested {
                layer.nodes.insert(unsupported.id.clone(), unsupported);
            } else {
                draft.nodes.insert(unsupported.id.clone(), unsupported);
            }
            draft.layers.insert(layer.id.clone(), layer);
            let before = serde_json::to_value(&draft).unwrap();
            assert!(
                validate_isolated_json_board(&draft)
                    .unwrap_err()
                    .contains("http_request")
            );
            assert_eq!(serde_json::to_value(&draft).unwrap(), before);

            let mut draft = return_board(json!(1));
            let mut layer = Layer::new("helper".into(), "helper".into(), LayerType::Function);
            let variable = Variable::new("state", VariableType::String, ValueType::Normal);
            if nested {
                layer.variables.insert(variable.id.clone(), variable);
            } else {
                draft.variables.insert(variable.id.clone(), variable);
            }
            draft.layers.insert(layer.id.clone(), layer);
            let before = serde_json::to_value(&draft).unwrap();
            assert!(
                validate_isolated_json_board(&draft)
                    .unwrap_err()
                    .contains("variables")
            );
            assert_eq!(serde_json::to_value(&draft).unwrap(), before);
        }

        let mut draft = return_board(json!(1));
        let mut layer = Layer::new("helper".into(), "helper".into(), LayerType::Function);
        layer.cache = Some(flow_like_runtime::flow::board::LayerCache {
            enabled: true,
            ..Default::default()
        });
        draft.layers.insert(layer.id.clone(), layer);
        let before = serde_json::to_value(&draft).unwrap();
        assert!(
            validate_isolated_json_board(&draft)
                .unwrap_err()
                .contains("caches")
        );
        assert_eq!(serde_json::to_value(&draft).unwrap(), before);
    }

    async fn test_source(source: &str, payload: Value) -> Result<DraftTestResult, String> {
        test_source_entry(source, "probe", payload).await
    }

    async fn test_source_entry(
        source: &str,
        entry: &str,
        payload: Value,
    ) -> Result<DraftTestResult, String> {
        use flow_like::flow::ast::apply_flowscript_to_board;
        prepare_and_test_draft_board(
            board(),
            entry,
            Some(payload),
            |mut board, state| async move {
                let catalog = state
                    .node_registry
                    .read()
                    .await
                    .get_nodes()
                    .map_err(|e| e.to_string())?;
                let applied =
                    apply_flowscript_to_board(&mut board, source, &catalog, state, None, false)
                        .await
                        .map_err(|e| e.to_string())?;
                if !applied.diagnostics.is_empty() {
                    return Err(applied.diagnostics.join("; "));
                }
                Ok(board)
            },
        )
        .await
    }

    #[tokio::test]
    async fn nested_ternary_source_executes_all_priority_boundaries() {
        const SOURCE: &str = r#"eventsGeneric priorityBand(payload: Struct, score: int) {
    const band = score < 50 ? "low" : (score < 80 ? "medium" : "high")
    const result = struct::set({ structIn: {}, field: "band", value: band })
    return result
}
"#;
        for (score, band) in [
            (-1, "low"),
            (49, "low"),
            (50, "medium"),
            (79, "medium"),
            (80, "high"),
        ] {
            let result = test_source_entry(SOURCE, "priorityBand", json!({"score": score}))
                .await
                .unwrap();
            assert_eq!(
                result.status,
                DraftTestStatus::Success,
                "score={score}: {result:?}"
            );
            assert!(result.errors.is_empty(), "score={score}: {result:?}");
            assert_eq!(result.output, Some(json!({"band": band})), "score={score}");
            assert_eq!(result.outputs, vec![json!({"band": band})], "score={score}");
        }
    }

    #[tokio::test]
    async fn compiler_binary_operator_table_executes_in_the_isolated_catalog() {
        for &(operator, operand, _, node_type) in flow_like::flow::ast::binary_operator_rows() {
            let (kind, a, b) = match operand {
                "String" => ("string", json!("Ab"), json!("aB")),
                "Boolean" => ("bool", json!(false), json!(false)),
                "Integer" => ("int", json!(6), json!(2)),
                "Float" => ("float", json!(4.0), json!(2.0)),
                other => panic!("add an operator execution fixture for {other}"),
            };
            let expected = match (operand, operator) {
                ("String", "==") => json!(false),
                ("String", "!=") => json!(true),
                ("String", "+") => json!("AbaB"),
                ("Boolean", "==") => json!(true),
                ("Boolean", "!=") => json!(false),
                ("Boolean", "&&" | "||" | "^") => json!(false),
                ("Integer" | "Float", "==" | "<" | "<=") => json!(false),
                ("Integer" | "Float", "!=" | ">" | ">=") => json!(true),
                ("Integer", "+") => json!(8),
                ("Integer", "-") => json!(4),
                ("Integer", "*") => json!(12),
                ("Integer", "/") => json!(3.0),
                ("Integer", "%") => json!(0),
                ("Integer", "**") => json!(36),
                ("Float", "+") => json!(6.0),
                ("Float", "-") => json!(2.0),
                ("Float", "*") => json!(8.0),
                ("Float", "/") => json!(2.0),
                ("Float", "**") => json!(16.0),
                other => panic!("add an expected result for {other:?}"),
            };
            let source = format!(
                "eventsGeneric probe(a: {kind}, b: {kind}) {{\n    return a {operator} b\n}}\n"
            );
            let result = test_source(&source, json!({"a": a, "b": b}))
                .await
                .unwrap_or_else(|error| panic!("{node_type}: {error}"));
            assert_eq!(
                result.status,
                DraftTestStatus::Success,
                "{node_type}: {result:?}"
            );
            assert!(result.errors.is_empty(), "{node_type}: {result:?}");
            assert_eq!(result.outputs, vec![expected], "{node_type}");
        }
    }

    #[tokio::test]
    async fn boolean_equality_and_xor_cover_the_complete_truth_table() {
        let source =
            "eventsGeneric probe(a: bool, b: bool) { return { equal: a == b, xor: a ^ b } }";
        for (a, b, equal, xor) in [
            (false, false, true, false),
            (false, true, false, true),
            (true, false, false, true),
            (true, true, true, false),
        ] {
            let result = test_source(source, json!({"a": a, "b": b})).await.unwrap();
            assert_eq!(
                result.status,
                DraftTestStatus::Success,
                "{a}/{b}: {result:?}"
            );
            assert_eq!(result.outputs, vec![json!({"equal": equal, "xor": xor})]);
        }
    }

    #[tokio::test]
    async fn numeric_comparisons_include_equal_and_adjacent_boundaries() {
        for (kind, a, b, expected) in [
            (
                "int",
                json!(-1),
                json!(0),
                json!({"ge": false, "le": true, "ne": true}),
            ),
            (
                "int",
                json!(0),
                json!(0),
                json!({"ge": true, "le": true, "ne": false}),
            ),
            (
                "int",
                json!(1),
                json!(0),
                json!({"ge": true, "le": false, "ne": true}),
            ),
            (
                "float",
                json!(1.25),
                json!(1.5),
                json!({"ge": false, "le": true, "ne": true}),
            ),
            (
                "float",
                json!(1.5),
                json!(1.5),
                json!({"ge": true, "le": true, "ne": false}),
            ),
            (
                "float",
                json!(1.75),
                json!(1.5),
                json!({"ge": true, "le": false, "ne": true}),
            ),
        ] {
            let source = format!(
                "eventsGeneric probe(a: {kind}, b: {kind}) {{ return {{ ge: a >= b, le: a <= b, ne: a != b }} }}"
            );
            let result = test_source(&source, json!({"a": a, "b": b})).await.unwrap();
            assert_eq!(
                result.status,
                DraftTestStatus::Success,
                "{kind}: {result:?}"
            );
            assert_eq!(result.outputs, vec![expected]);
        }
    }

    #[tokio::test]
    async fn focused_square_helper_runs_with_distinct_signed_inputs() {
        let source = r#"function square(value: int): (area: int) {
    const area = value * value
    return area
}
eventsGeneric tileAreas(sideA: int, sideB: int) {
    const squareA = square({ value: sideA })
    const squareB = square({ value: sideB })
    return [squareA, squareB]
}
"#;
        for (a, b, expected) in [(2, -3, json!([4, 9])), (-4, 0, json!([16, 0]))] {
            let result = test_source_entry(source, "tileAreas", json!({"sideA": a, "sideB": b}))
                .await
                .unwrap();
            assert_eq!(result.status, DraftTestStatus::Success, "{result:?}");
            assert_eq!(result.outputs, vec![expected]);
            assert_eq!(
                result.helper_calls.values().copied().collect::<Vec<_>>(),
                vec![2]
            );
        }
    }

    #[tokio::test]
    async fn arithmetic_guards_preserve_valid_signed_and_fractional_results() {
        for (kind, op, a, b, expected) in [
            ("int", "*", json!(i64::MAX), json!(1), json!(i64::MAX)),
            ("int", "*", json!(i64::MIN), json!(1), json!(i64::MIN)),
            ("int", "%", json!(i64::MIN), json!(-2), json!(0)),
            ("int", "%", json!(-7), json!(2), json!(-1)),
            ("int", "/", json!(-7), json!(2), json!(-3.5)),
            ("int", "**", json!(-2), json!(3), json!(-8)),
            ("int", "**", json!(1), json!(u32::MAX), json!(1)),
            ("float", "+", json!(1.25), json!(0.5), json!(1.75)),
            ("float", "-", json!(1.25), json!(0.5), json!(0.75)),
            ("float", "*", json!(1.25), json!(0.5), json!(0.625)),
            ("float", "/", json!(1.25), json!(0.5), json!(2.5)),
            ("float", "**", json!(4.0), json!(0.5), json!(2.0)),
        ] {
            let source = format!("eventsGeneric probe(a: {kind}, b: {kind}) {{ return a {op} b }}");
            let result = test_source(&source, json!({"a": a, "b": b})).await.unwrap();
            assert_eq!(
                result.status,
                DraftTestStatus::Success,
                "{kind}/{op}: {result:?}"
            );
            assert_eq!(result.outputs, vec![expected], "{kind}/{op}");
        }
    }

    #[tokio::test]
    async fn arithmetic_errors_cannot_become_wrapped_numbers_or_json_null_successes() {
        for (kind, op, a, b, error) in [
            ("int", "*", json!(i64::MAX), json!(2), "overflow"),
            ("int", "%", json!(i64::MIN), json!(-1), "overflow"),
            ("int", "%", json!(3), json!(0), "zero"),
            ("int", "/", json!(3), json!(0), "zero"),
            ("int", "**", json!(2), json!(63), "overflow"),
            ("int", "**", json!(2), json!(-1), "exponent"),
            (
                "int",
                "**",
                json!(1),
                json!(i64::from(u32::MAX) + 1),
                "exponent",
            ),
            ("float", "+", json!(1e308), json!(1e308), "nonfinite"),
            ("float", "-", json!(1e308), json!(-1e308), "nonfinite"),
            ("float", "*", json!(1e308), json!(2.0), "nonfinite"),
            ("float", "/", json!(1.0), json!(0.0), "zero"),
            ("float", "/", json!(1e308), json!(1e-308), "nonfinite"),
            ("float", "**", json!(-1.0), json!(0.5), "nonfinite"),
            ("float", "**", json!(1e308), json!(2.0), "nonfinite"),
        ] {
            let source = format!("eventsGeneric probe(a: {kind}, b: {kind}) {{ return a {op} b }}");
            let result = test_source(&source, json!({"a": a, "b": b})).await.unwrap();
            assert_eq!(
                result.status,
                DraftTestStatus::Failed,
                "{kind}/{op}: {result:?}"
            );
            assert!(
                result.errors.iter().any(|message| message.contains(error)),
                "{kind}/{op}: {result:?}"
            );
            assert!(result.outputs.is_empty(), "{kind}/{op}: {result:?}");
        }
    }

    #[tokio::test]
    async fn string_operators_preserve_values_and_bound_concatenation_growth() {
        let source = "eventsGeneric probe(a: string, b: string) { return { joined: a + b, different: a != b } }";
        for (a, b, expected) in [
            ("", "", json!({"joined": "", "different": false})),
            ("Ab", "aB", json!({"joined": "AbaB", "different": true})),
            ("é", "雪", json!({"joined": "é雪", "different": true})),
        ] {
            let result = test_source(source, json!({"a": a, "b": b})).await.unwrap();
            assert_eq!(result.status, DraftTestStatus::Success, "{result:?}");
            assert_eq!(result.outputs, vec![expected]);
        }
        let result = test_source(
            "eventsGeneric probe(text: string) { return text + text }",
            json!({"text": "x".repeat(9000)}),
        )
        .await
        .unwrap();
        assert_eq!(result.status, DraftTestStatus::LimitExceeded, "{result:?}");
        assert!(
            result
                .errors
                .iter()
                .any(|error| error.contains("concatenated string")),
            "{result:?}"
        );
        assert!(result.outputs.is_empty());

        let blocked = test_source(
            "eventsGeneric probe(name: string) { return `hello ${name}` }",
            json!({"name": "Ada"}),
        )
        .await
        .expect_err("template substitution remains outside the trusted catalog");
        assert!(blocked.contains("string_format"), "{blocked}");
    }

    #[tokio::test]
    async fn pure_helpers_keep_repeated_call_arguments_and_outputs_separate() {
        let result = test_source(
            r#"function normalize(value: string): (result: string) {
    const result = value.trim()
    return result
}
eventsGeneric probe(left: string, right: string) {
    const leftResult = normalize({ value: left })
    const rightResult = normalize({ value: right })
    return { left: leftResult, right: rightResult }
}
"#,
            json!({"left": "  first  ", "right": " second "}),
        )
        .await
        .unwrap();
        assert_eq!(result.status, DraftTestStatus::Success, "{result:?}");
        assert_eq!(
            result.output,
            Some(json!({"left": "first", "right": "second"}))
        );
        assert_eq!(
            result.helper_calls.values().copied().collect::<Vec<_>>(),
            vec![2]
        );
    }

    #[tokio::test]
    async fn impure_helpers_return_computed_objects_and_arrays() {
        let result = test_source(
            r#"function makePayload(title: string, revision: int): (payload: Struct) {
    const cleaned = title.trim()
    const row = { title: cleaned, revision: revision + 1 }
    const rows = [row]
    return { rows: rows }
}
eventsGeneric probe(title: string, revision: int) {
    return makePayload({ title: title, revision: revision })
}
"#,
            json!({"title": "  Review  ", "revision": 2}),
        )
        .await
        .unwrap();
        assert_eq!(result.status, DraftTestStatus::Success, "{result:?}");
        assert_eq!(
            result.output,
            Some(json!({"rows": [{"title": "Review", "revision": 3}]}))
        );
    }

    #[tokio::test]
    async fn nested_helpers_execute_within_a_named_generic_entry() {
        let result = test_source(
            r#"function clean(value: string): (result: string) {
    const result = value.trim()
    return result
}
function normalize(value: string): (result: string) {
    const cleaned = clean({ value: value })
    const result = cleaned.toLower()
    return result
}
eventsGeneric probe(text: string) {
    return normalize({ value: text })
}
"#,
            json!({"text": "  READY  "}),
        )
        .await
        .unwrap();
        assert_eq!(result.status, DraftTestStatus::Success, "{result:?}");
        assert_eq!(result.output, Some(json!("ready")));
        assert_eq!(
            result.helper_calls.values().copied().collect::<Vec<_>>(),
            vec![1, 1]
        );
    }

    #[tokio::test]
    async fn branches_can_call_the_same_helper_with_different_arguments() {
        let source = r#"function normalize(value: string): (result: string) {
    const result = value.trim()
    return result
}
eventsGeneric probe(left: string, right: string, chooseLeft: bool) {
    if (chooseLeft) {
        return normalize({ value: left })
    } else {
        return normalize({ value: right })
    }
}
"#;
        for (choose_left, expected) in [(true, "first"), (false, "second")] {
            let result = test_source(
                source,
                json!({"left": " first ", "right": " second ", "chooseLeft": choose_left}),
            )
            .await
            .unwrap();
            assert_eq!(result.status, DraftTestStatus::Success, "{result:?}");
            assert_eq!(result.output, Some(json!(expected)));
            assert_eq!(
                result.helper_calls.values().copied().collect::<Vec<_>>(),
                vec![1]
            );
        }
    }

    #[tokio::test]
    async fn declared_but_uncalled_helpers_produce_no_call_evidence() {
        let result = test_source(
            r#"function unused(value: string): (result: string) {
    const result = value.trim()
    return result
}
eventsGeneric probe() {
    return "done"
}
"#,
            json!({}),
        )
        .await
        .unwrap();
        assert_eq!(result.status, DraftTestStatus::Success, "{result:?}");
        assert_eq!(result.output, Some(json!("done")));
        assert!(result.helper_calls.is_empty());
    }

    #[tokio::test]
    async fn helper_errors_and_scoped_output_limits_fail_the_whole_test() {
        let failed = test_source(
            r#"function guarded(value: string): (result: string) {
    test::assert({ condition: false })
    const result = value.trim()
    return result
}
eventsGeneric probe(text: string) {
    return guarded({ value: text })
}
"#,
            json!({"text": "hello"}),
        )
        .await
        .unwrap();
        assert_eq!(failed.status, DraftTestStatus::Failed, "{failed:?}");
        assert!(failed.errors.iter().any(|e| e.contains("ASSERT_FAIL")));
        assert!(failed.output.is_none());

        let limited = test_source(
            r#"function expand(value: string): (result: string) {
    const result = value.toUpper()
    return result
}
eventsGeneric probe(text: string) {
    return expand({ value: text })
}
"#,
            json!({"text": "ΐ".repeat(4000)}),
        )
        .await
        .unwrap();
        assert_eq!(
            limited.status,
            DraftTestStatus::LimitExceeded,
            "{limited:?}"
        );
        assert!(limited.output.is_none());
    }

    #[tokio::test]
    async fn nested_helper_calls_share_the_invocation_budget() {
        let mut source = "function f0(value: string): (result: string) {\n    const result = value.trim()\n    return result\n}\n".to_string();
        for index in 1..8 {
            let previous = index - 1;
            source.push_str(&format!("function f{index}(value: string): (result: string) {{\n    const first = f{previous}({{ value: value }})\n    const result = f{previous}({{ value: first }})\n    return result\n}}\n"));
        }
        source.push_str("eventsGeneric probe(text: string) {\n    return f7({ value: text })\n}\n");
        let result = test_source(&source, json!({"text": " text "}))
            .await
            .unwrap();
        assert_eq!(result.status, DraftTestStatus::LimitExceeded, "{result:?}");
        assert_eq!(result.nodes_executed, MAX_INVOCATIONS);
        assert!(result.output.is_none());
    }

    #[test]
    fn helper_targets_are_static_local_acyclic_and_keep_wires_in_scope() {
        let mut draft = return_board(json!(1));
        draft.layers.insert(
            "helper".into(),
            Layer::new("helper".into(), "helper".into(), LayerType::Function),
        );
        add(&mut draft, "control_call_function", "call");
        literal(
            &mut draft,
            "call",
            "function_layer_id",
            json!("remote-board/helper"),
        );
        assert!(
            validate_board(&draft)
                .unwrap_err()
                .contains("on this board")
        );

        literal(&mut draft, "call", "function_layer_id", json!("helper"));
        wire(
            &mut draft,
            "result",
            "response",
            "call",
            "function_layer_id",
        );
        assert!(
            validate_board(&draft)
                .unwrap_err()
                .contains("statically bound")
        );
        draft
            .nodes
            .get_mut("call")
            .unwrap()
            .pins
            .values_mut()
            .for_each(|pin| pin.depends_on.clear());
        draft
            .nodes
            .get_mut("result")
            .unwrap()
            .pins
            .values_mut()
            .for_each(|pin| pin.connected_to.clear());

        let recursive_call = draft.nodes.remove("call").unwrap();
        draft
            .layers
            .get_mut("helper")
            .unwrap()
            .nodes
            .insert(recursive_call.id.clone(), recursive_call);
        assert!(validate_board(&draft).unwrap_err().contains("recursive"));
        draft.layers.get_mut("helper").unwrap().nodes.clear();
        add(&mut draft, "string_trim", "inside");
        draft.nodes.get_mut("inside").unwrap().layer = Some("helper".into());
        wire(&mut draft, "inside", "trimmed_string", "result", "response");
        assert!(
            validate_board(&draft)
                .unwrap_err()
                .contains("across helper")
        );
    }

    #[test]
    fn unused_helpers_cannot_hide_unsupported_nodes() {
        let mut draft = return_board(json!(1));
        let mut layer = Layer::new("helper".into(), "helper".into(), LayerType::Function);
        let unsafe_node = Node::new("http_request", "Request", "", "");
        layer.nodes.insert(unsafe_node.id.clone(), unsafe_node);
        draft.layers.insert(layer.id.clone(), layer);
        assert!(validate_board(&draft).unwrap_err().contains("http_request"));
        let helper = draft.layers.get_mut("helper").unwrap();
        helper.nodes.clear();
        helper.cache = Some(flow_like_runtime::flow::board::LayerCache {
            enabled: true,
            ..Default::default()
        });
        assert!(validate_board(&draft).unwrap_err().contains("caches"));
    }

    #[tokio::test]
    async fn preparation_returns_a_detached_board_without_executing_it() {
        let source = r#"eventsGeneric probe() {
    test::assert({ condition: false })
    return "unreachable"
}
"#;
        let prepared = prepare_draft_board(board(), |mut board, state| async move {
            let catalog = state
                .node_registry
                .read()
                .await
                .get_nodes()
                .map_err(|e| e.to_string())?;
            let applied = flow_like::flow::ast::apply_flowscript_to_board(
                &mut board,
                source,
                &catalog,
                state.clone(),
                None,
                false,
            )
            .await
            .map_err(|e| e.to_string())?;
            if !applied.diagnostics.is_empty() {
                return Err(applied.diagnostics.join("; "));
            }
            board.app_state = Some(state);
            Ok(board)
        })
        .await
        .unwrap();
        assert!(prepared.app_state.is_none());
        assert!(prepared.logic_nodes.is_empty());
        assert!(
            prepared
                .nodes
                .values()
                .all(|node| node.pins.values().all(|pin| pin.value.is_none()))
        );
        let executed = test_draft_board(prepared, "probe", None).await.unwrap();
        assert_eq!(executed.status, DraftTestStatus::Failed, "{executed:?}");
        assert!(executed.errors.iter().any(|e| e.contains("ASSERT_FAIL")));
    }

    #[tokio::test]
    async fn native_transform_fails_then_passes_after_repair() {
        let mut board = board();
        add(&mut board, "events_generic", "test");
        add(&mut board, "struct_get", "field");
        add(&mut board, "string_trim", "trim");
        add(&mut board, "events_generic_return_result", "result");
        literal(&mut board, "field", "field", json!("wrong_field"));
        wire(&mut board, "test", "exec_out", "result", "exec_in");
        wire(&mut board, "test", "payload", "field", "struct");
        wire(&mut board, "field", "value", "trim", "string");
        wire(&mut board, "trim", "trimmed_string", "result", "response");

        let payload = Some(json!({"text": "  hello  "}));
        let failed = test_draft_board(board.clone(), "test", payload.clone())
            .await
            .unwrap();
        assert_eq!(failed.status, DraftTestStatus::Failed, "{failed:?}");
        assert!(!failed.errors.is_empty());

        let passed =
            prepare_and_test_draft_board(board, "test", payload, |mut board, state| async move {
                assert_eq!(
                    state.node_registry.read().await.get_nodes().unwrap().len(),
                    supported_node_names().len()
                );
                literal(&mut board, "field", "field", json!("text"));
                Ok(board)
            })
            .await
            .unwrap();
        assert_eq!(passed.status, DraftTestStatus::Success, "{passed:?}");
        assert_eq!(passed.output, Some(json!("hello")));
        assert_eq!(passed.outputs, vec![json!("hello")]);
        assert!(passed.nodes_executed >= 4);
        assert!(passed.errors.is_empty());
    }

    #[tokio::test]
    async fn retained_source_compiles_tests_and_repairs_the_exact_revision() {
        use flow_like::flow::{
            ast::apply_board_commands_to_board,
            copilot::{
                CheckFlowScriptArgs, FlowIrDraftMode, FlowIrDraftStore, PatchFlowScriptArgs,
                WriteFlowScriptArgs, node_to_metadata,
            },
        };
        let base = board();
        let original = serde_json::to_value(&base).unwrap();
        let metadata: Vec<_> = native_nodes()
            .iter()
            .map(|n| node_to_metadata(&n.get_node()))
            .collect();
        let store = FlowIrDraftStore::new();
        let binding = store.bind_request_acceptance_contract(
            &base.id,
            "Trim the supplied text and return a row containing its label.",
        );
        let source = r#"eventsGeneric normalize(text: string) {
    const cleaned = text.toUpper()
    const row = { label: cleaned }
    const rows = [row]
    return { rows: rows }
}
"#;
        let written = store.write_flowscript_with_acceptance_binding(
            &base,
            &metadata,
            WriteFlowScriptArgs {
                draft_id: "normalize-draft".into(),
                replace_existing: false,
                mode: FlowIrDraftMode::Additive,
                source: source.into(),
                allow_scope_reduction: false,
            },
            &binding,
        );
        assert_eq!(written.revision, Some(0), "{written:?}");
        let mut fingerprints = Vec::new();
        let expected = json!({"rows": [{"label": "hello"}]});
        for revision in 0..2 {
            let snapshot = store
                .prepare_flowscript_test(
                    &base,
                    &metadata,
                    CheckFlowScriptArgs {
                        draft_id: "normalize-draft".into(),
                        expected_revision: revision,
                    },
                    &binding,
                )
                .unwrap();
            fingerprints.push(snapshot.source_fingerprint.clone());
            let result = prepare_and_test_draft_board(
                base.clone(),
                "normalize",
                Some(json!({"text": "  hello  "})),
                move |mut board, state| async move {
                    let catalog = state
                        .node_registry
                        .read()
                        .await
                        .get_nodes()
                        .map_err(|e| e.to_string())?;
                    let applied = apply_board_commands_to_board(
                        &mut board,
                        snapshot.commands,
                        &catalog,
                        state,
                        None,
                    )
                    .await
                    .map_err(|e| e.to_string())?;
                    if !applied.diagnostics.is_empty() {
                        return Err(applied.diagnostics.join("; "));
                    }
                    Ok(board)
                },
            )
            .await
            .unwrap();
            assert_eq!(result.status, DraftTestStatus::Success, "{result:?}");
            let passed = result.output == Some(expected.clone()) && result.outputs.len() == 1;
            assert_eq!(passed, revision == 1, "revision {revision}: {result:?}");
            if revision == 0 {
                let patched = store.patch_flowscript_with_acceptance_binding(
                    &base,
                    &metadata,
                    PatchFlowScriptArgs {
                        draft_id: "normalize-draft".into(),
                        expected_revision: 0,
                        old_text: "text.toUpper()".into(),
                        new_text: "text.trim()".into(),
                        allow_scope_reduction: false,
                    },
                    &binding,
                );
                assert_eq!(patched.revision, Some(1), "{patched:?}");
                assert!(
                    store
                        .prepare_flowscript_test(
                            &base,
                            &metadata,
                            CheckFlowScriptArgs {
                                draft_id: "normalize-draft".into(),
                                expected_revision: 0,
                            },
                            &binding
                        )
                        .is_err()
                );
            }
        }
        assert_ne!(fingerprints[0], fingerprints[1]);
        assert_eq!(serde_json::to_value(&base).unwrap(), original);
    }

    #[tokio::test]
    async fn assertion_failure_is_runtime_evidence() {
        let mut board = return_board(json!("unreachable"));
        add(&mut board, "flow_assert", "assert");
        // Disconnect the result so a failed assertion cannot produce the expected value.
        board
            .nodes
            .get_mut("test")
            .unwrap()
            .pins
            .values_mut()
            .for_each(|p| p.connected_to.clear());
        board
            .nodes
            .get_mut("result")
            .unwrap()
            .pins
            .values_mut()
            .for_each(|p| p.depends_on.clear());
        wire(&mut board, "test", "exec_out", "assert", "exec_in");
        wire(&mut board, "assert", "exec_out", "result", "exec_in");
        let result = test_draft_board(board, "test", None).await.unwrap();
        assert_eq!(result.status, DraftTestStatus::Failed, "{result:?}");
        assert!(result.output.is_none());
        assert!(result.errors.iter().any(|e| e.contains("ASSERT_FAIL")));
    }

    #[tokio::test]
    async fn handled_dependency_failure_cannot_certify_a_successful_fallback() {
        let mut draft = board();
        add(&mut draft, "events_simple", "test");
        add(&mut draft, "control_branch", "driver");
        add(&mut draft, "bool_not", "a");
        add(&mut draft, "bool_not", "b");
        add(&mut draft, "events_generic_return_result", "fallback");
        literal(&mut draft, "fallback", "response", json!(42));
        let driver = draft.nodes.get_mut("driver").unwrap();
        driver.add_output_pin("auto_handle_error", "On Error", "", VariableType::Execution);
        driver.add_output_pin(
            "auto_handle_error_string",
            "Error",
            "",
            VariableType::String,
        );
        wire(&mut draft, "test", "exec_out", "driver", "exec_in");
        wire(&mut draft, "a", "result", "b", "boolean");
        wire(&mut draft, "b", "result", "a", "boolean");
        wire(&mut draft, "a", "result", "driver", "condition");
        wire(
            &mut draft,
            "driver",
            "auto_handle_error",
            "fallback",
            "exec_in",
        );
        let result = test_draft_board(draft, "test", None).await.unwrap();
        assert_eq!(result.status, DraftTestStatus::Failed, "{result:?}");
        assert_eq!(result.output, Some(json!(42)), "{result:?}");
        assert!(
            result.errors.iter().any(|e| e.contains("Cycle detected")),
            "{result:?}"
        );
    }

    #[tokio::test]
    async fn exactly_one_runtime_result_is_required() {
        let mut empty = board();
        add(&mut empty, "events_simple", "test");
        let result = test_draft_board(empty, "test", None).await.unwrap();
        assert_eq!(result.status, DraftTestStatus::Failed, "{result:?}");
        assert!(result.output.is_none());
        assert!(result.errors.iter().any(|e| e.contains("emitted 0")));

        let mut multiple = return_board(json!(1));
        add(&mut multiple, "events_generic_return_result", "second");
        literal(&mut multiple, "second", "response", json!(2));
        wire(&mut multiple, "test", "exec_out", "second", "exec_in");
        let result = test_draft_board(multiple, "test", None).await.unwrap();
        assert_eq!(result.status, DraftTestStatus::Failed, "{result:?}");
        assert!(result.output.is_none());
        assert_eq!(result.outputs.len(), 2);
    }

    #[tokio::test]
    async fn rejects_unsupported_nodes_before_and_after_preparation_including_layers() {
        let mut unsafe_board = return_board(json!(1));
        let mut layer = Layer::new("hidden".into(), "hidden".into(), LayerType::Collapsed);
        let mut unsafe_node = Node::new("http_request", "Request", "", "");
        unsafe_node.id = "unsafe".into();
        layer.nodes.insert(unsafe_node.id.clone(), unsafe_node);
        unsafe_board.layers.insert(layer.id.clone(), layer);
        let called = Arc::new(AtomicBool::new(false));
        let observed = called.clone();
        let result =
            prepare_and_test_draft_board(unsafe_board, "test", None, move |board, _| async move {
                observed.store(true, Ordering::SeqCst);
                Ok(board)
            })
            .await;
        assert!(result.unwrap_err().contains("http_request"));
        assert!(!called.load(Ordering::SeqCst));

        let result = prepare_and_test_draft_board(
            return_board(json!(1)),
            "test",
            None,
            |mut board, _| async move {
                board.nodes.get_mut("result").unwrap().name = "http_request".into();
                Ok(board)
            },
        )
        .await;
        assert!(result.unwrap_err().contains("http_request"));
    }

    #[test]
    fn rejects_macro_layers_and_non_json_pins() {
        let mut draft = return_board(json!(1));
        draft.layers.insert(
            "function".into(),
            Layer::new("function".into(), "Function".into(), LayerType::Macro),
        );
        assert!(validate_board(&draft).unwrap_err().contains("macros"));
        draft.layers.clear();
        draft
            .nodes
            .get_mut("result")
            .unwrap()
            .pins
            .values_mut()
            .find(|p| p.name == "response")
            .unwrap()
            .data_type = VariableType::PathBuf;
        assert!(validate_board(&draft).unwrap_err().contains("JSON"));
    }

    #[test]
    fn builtin_name_does_not_authorize_a_wasm_implementation() {
        let mut draft = return_board(json!(1));
        draft.nodes.get_mut("result").unwrap().wasm =
            Some(flow_like_runtime::flow::node::NodeWasm {
                package_id: "untrusted".into(),
                permissions: vec![],
            });
        assert!(validate_board(&draft).unwrap_err().contains("unsupported"));
    }

    #[tokio::test]
    async fn execution_cycles_cannot_run_indefinitely() {
        let mut draft = board();
        add(&mut draft, "events_simple", "test");
        add(&mut draft, "control_branch", "cycle");
        wire(&mut draft, "test", "exec_out", "cycle", "exec_in");
        wire(&mut draft, "cycle", "true", "cycle", "exec_in");
        let result = test_draft_board(draft, "test", None).await.unwrap();
        assert_eq!(result.status, DraftTestStatus::Failed, "{result:?}");
        assert!(result.nodes_executed <= MAX_INVOCATIONS);
        assert!(result.output.is_none());
    }

    #[tokio::test]
    async fn repeated_dependency_evaluation_stops_at_the_invocation_budget() {
        let mut draft = board();
        add(&mut draft, "events_simple", "test");
        for index in 0..20 {
            let id = format!("not{index}");
            add(&mut draft, "bool_not", &id);
            if index == 0 {
                literal(&mut draft, &id, "boolean", json!(true));
            } else {
                wire(
                    &mut draft,
                    &format!("not{}", index - 1),
                    "result",
                    &id,
                    "boolean",
                );
            }
        }
        for index in 0..20 {
            let id = format!("assert{index}");
            add(&mut draft, "flow_assert", &id);
            wire(&mut draft, "not19", "result", &id, "condition");
            let previous = if index == 0 {
                "test".into()
            } else {
                format!("assert{}", index - 1)
            };
            wire(&mut draft, &previous, "exec_out", &id, "exec_in");
        }
        let result = test_draft_board(draft, "test", None).await.unwrap();
        assert_eq!(result.status, DraftTestStatus::LimitExceeded, "{result:?}");
        assert_eq!(result.nodes_executed, MAX_INVOCATIONS);
        assert!(result.output.is_none());
    }

    #[tokio::test]
    async fn swallowed_callback_errors_cannot_turn_output_limit_into_success() {
        let mut draft = board();
        add(&mut draft, "events_simple", "test");
        for index in 0..=MAX_OUTPUTS {
            let id = format!("result{index}");
            add(&mut draft, "events_generic_return_result", &id);
            literal(&mut draft, &id, "response", json!(index));
            wire(&mut draft, "test", "exec_out", &id, "exec_in");
        }
        let result = test_draft_board(draft, "test", None).await.unwrap();
        assert_eq!(result.status, DraftTestStatus::LimitExceeded, "{result:?}");
        assert_eq!(result.outputs.len(), MAX_OUTPUTS);
        assert!(result.output.is_none());
    }

    struct HostOverride(Arc<AtomicBool>);

    #[async_trait]
    impl NodeLogic for HostOverride {
        fn get_node(&self) -> Node {
            native_nodes()
                .into_iter()
                .find(|n| n.get_node().name == "events_simple")
                .unwrap()
                .get_node()
        }
        async fn run(&self, _: &mut ExecutionContext) -> flow_like_types::Result<()> {
            self.0.store(true, Ordering::SeqCst);
            Err(flow_like_types::anyhow!(
                "host implementation must never execute"
            ))
        }
    }

    #[tokio::test]
    async fn discards_host_implementations_state_and_runtime_pin_values() {
        let called = Arc::new(AtomicBool::new(false));
        let mut draft = return_board(json!("isolated"));
        let host_store = Arc::new(InMemory::new());
        let host = Arc::new(FlowLikeState::new(
            FlowLikeConfig::with_default_store(FlowLikeStore::Memory(host_store.clone())),
            HTTPClient::new_without_refetch(),
        ));
        host.node_registry
            .write()
            .await
            .push_node(Arc::new(HostOverride(called.clone())));
        draft.app_state = Some(host);
        draft
            .logic_nodes
            .insert("test".into(), Arc::new(HostOverride(called.clone())));
        let pin = draft
            .nodes
            .get_mut("result")
            .unwrap()
            .pins
            .values_mut()
            .find(|p| p.name == "response")
            .unwrap();
        pin.value = Some(Arc::new(flow_like_types::sync::Mutex::new(json!(
            "host secret"
        ))));
        let result =
            prepare_and_test_draft_board(draft, "test", None, move |board, state| async move {
                assert!(board.app_state.is_none());
                assert!(board.logic_nodes.is_empty());
                assert!(
                    board
                        .nodes
                        .values()
                        .all(|n| n.pins.values().all(|p| p.value.is_none()))
                );
                let config = state.config.read().await;
                let Some(FlowLikeStore::Memory(store)) = &config.stores.app_storage_store else {
                    panic!("expected isolated memory store")
                };
                assert!(!Arc::ptr_eq(&host_store, store));
                assert!(config.stores.log_store.is_none());
                Ok(board)
            })
            .await
            .unwrap();
        assert_eq!(result.status, DraftTestStatus::Success, "{result:?}");
        assert_eq!(result.output, Some(json!("isolated")));
        assert!(!called.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn rejects_oversized_inputs_and_native_integer_overflow() {
        let result = test_draft_board(
            return_board(json!(1)),
            "test",
            Some(json!("x".repeat(MAX_VALUE_BYTES))),
        )
        .await;
        assert!(result.is_err());

        let mut draft = return_board(json!(1));
        add(&mut draft, "int_add", "add");
        literal(&mut draft, "add", "integer1", json!(i64::MAX));
        literal(&mut draft, "add", "integer2", json!(1));
        wire(&mut draft, "add", "sum", "result", "response");
        let result = test_draft_board(draft, "test", None).await.unwrap();
        assert_eq!(result.status, DraftTestStatus::Failed, "{result:?}");
        assert!(result.errors.iter().any(|e| e.contains("overflow")));
    }

    #[tokio::test]
    async fn native_output_growth_cannot_bypass_value_limit() {
        let mut draft = return_board(json!(1));
        add(&mut draft, "string_to_upper", "upper");
        // This Greek character expands from two UTF-8 bytes to six on uppercasing.
        literal(&mut draft, "upper", "string", json!("ΐ".repeat(4000)));
        wire(
            &mut draft,
            "upper",
            "uppercase_string",
            "result",
            "response",
        );
        let result = test_draft_board(draft, "test", None).await.unwrap();
        assert_eq!(result.status, DraftTestStatus::LimitExceeded, "{result:?}");
        assert!(result.output.is_none());
    }

    #[tokio::test]
    async fn output_limit_stops_native_dispatch_through_an_error_handler() {
        let mut draft = board();
        add(&mut draft, "events_simple", "test");
        add(&mut draft, "string_to_upper", "upper");
        add(&mut draft, "events_generic_return_result", "fallback");
        literal(&mut draft, "upper", "string", json!("ΐ".repeat(4000)));
        literal(
            &mut draft,
            "fallback",
            "response",
            json!("must not execute"),
        );
        let upper = draft.nodes.get_mut("upper").unwrap();
        upper.add_input_pin("exec_in", "Input", "", VariableType::Execution);
        upper.add_output_pin("auto_handle_error", "On Error", "", VariableType::Execution);
        upper.add_output_pin(
            "auto_handle_error_string",
            "Error",
            "",
            VariableType::String,
        );
        wire(&mut draft, "test", "exec_out", "upper", "exec_in");
        wire(
            &mut draft,
            "upper",
            "auto_handle_error",
            "fallback",
            "exec_in",
        );

        let result = test_draft_board(draft, "test", None).await.unwrap();
        assert_eq!(result.status, DraftTestStatus::LimitExceeded, "{result:?}");
        assert!(result.outputs.is_empty(), "{result:?}");
        assert_eq!(result.nodes_executed, 2, "{result:?}");
    }

    #[tokio::test]
    async fn preparation_deadline_is_enforced() {
        let result = prepare_and_test_draft_board(return_board(json!(1)), "test", None, |_, _| {
            std::future::pending()
        })
        .await
        .unwrap();
        assert_eq!(result.status, DraftTestStatus::Timeout, "{result:?}");
        assert_eq!(result.nodes_executed, 0);
        assert!(result.output.is_none());
    }
}
