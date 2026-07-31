use crate::flow::{
    node::{Node, NodeLogic, NodeState},
    pin::PinType,
    utils::{InlineVisitedPins, evaluate_pin_value},
    variable::VariableType,
};
use ahash::{AHashMap, AHashSet};
use flow_like_types::{Value, json::json, sync::Mutex, utils::ptr_key};
use std::sync::{Arc, Weak, atomic::AtomicU64};

use super::{LogLevel, context::ExecutionContext, internal_pin::InternalPin, log::LogMessage};

#[derive(Debug)]
pub enum InternalNodeError {
    DependencyFailed(String),
    ExecutionFailed(String),
    PinNotReady(String),
}

impl InternalNodeError {
    fn into_message(self) -> String {
        match self {
            InternalNodeError::DependencyFailed(message)
            | InternalNodeError::ExecutionFailed(message)
            | InternalNodeError::PinNotReady(message) => message,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::{async_trait, tokio};

    #[derive(Default)]
    struct NoopLogic;

    #[async_trait]
    impl NodeLogic for NoopLogic {
        fn get_node(&self) -> Node {
            Node::new("noop", "Noop", "Noop", "Tests")
        }

        async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
            Ok(())
        }
    }

    fn internal_node(node: Node) -> Arc<InternalNode> {
        let mut pins = AHashMap::new();
        let mut name_cache: AHashMap<String, Vec<Arc<InternalPin>>> = AHashMap::new();

        for pin in node.pins.values() {
            let internal_pin = Arc::new(InternalPin::new(pin, false));
            name_cache
                .entry(pin.name.clone())
                .or_default()
                .push(internal_pin.clone());
            pins.insert(pin.id.clone(), internal_pin);
        }

        let internal = Arc::new(InternalNode::new(
            node,
            pins,
            Arc::new(NoopLogic),
            name_cache,
        ));
        for pin in internal.pins.values() {
            pin.init_node(Arc::downgrade(&internal));
        }
        internal
    }

    #[tokio::test]
    async fn immutable_name_cache_handles_duplicate_names_and_misses() {
        let mut node = Node::new("target", "Target", "Target", "Tests");
        node.add_input_pin("exec_in", "First", "First", VariableType::Execution);
        node.add_input_pin("exec_in", "Second", "Second", VariableType::Execution);
        let internal = internal_node(node);

        let pins = internal
            .get_pins_by_name("exec_in")
            .await
            .expect("duplicate-name pins should be cached");
        assert_eq!(pins.len(), 2);

        let cache_len = internal.pin_name_cache.len();
        assert!(internal.get_pin_by_name("missing").await.is_err());
        assert!(internal.get_pin_by_name("missing").await.is_err());
        assert_eq!(internal.pin_name_cache.len(), cache_len);
    }

    #[test]
    fn single_exec_output_merges_multiple_pins_for_one_target() {
        let mut target_node = Node::new("target", "Target", "Target", "Tests");
        target_node.add_input_pin("exec_in", "First", "First", VariableType::Execution);
        target_node.add_input_pin("exec_in", "Second", "Second", VariableType::Execution);
        let target = internal_node(target_node);
        let target_pins: Vec<_> = target.pins.values().cloned().collect();

        let mut source_node = Node::new("source", "Source", "Source", "Tests");
        let source_pin = source_node
            .add_output_pin("exec_out", "Output", "Output", VariableType::Execution)
            .clone();
        let source_pin = Arc::new(InternalPin::new(&source_pin, false));
        source_pin.init_connected_to(target_pins.iter().map(Arc::downgrade).collect());

        let targets = targets_for_single_exec_output(&source_pin);
        assert_eq!(targets.len(), 1);
        assert!(Arc::ptr_eq(&targets[0].node, &target));
        assert_eq!(targets[0].through_pins.len(), 2);
    }
}

#[derive(Clone)]
pub struct ExecutionTarget {
    pub node: Arc<InternalNode>,
    pub through_pins: Vec<Arc<InternalPin>>,
}

impl ExecutionTarget {
    async fn into_sub_context(&self, ctx: &mut ExecutionContext) -> ExecutionContext {
        let mut sub = ctx.create_sub_context(&self.node).await;
        sub.started_by = if self.through_pins.is_empty() {
            None
        } else {
            Some(self.through_pins.clone())
        };
        sub
    }
}

#[inline]
fn logs_at(ctx: &ExecutionContext, level: LogLevel) -> bool {
    level >= ctx.log_level
}

#[inline]
fn log_debug(ctx: &mut ExecutionContext, message: impl FnOnce() -> String) {
    if logs_at(ctx, LogLevel::Debug) {
        ctx.log_message(&message(), LogLevel::Debug);
    }
}

#[inline]
fn start_debug_log(ctx: &ExecutionContext, message: impl FnOnce() -> String) -> Option<LogMessage> {
    logs_at(ctx, LogLevel::Debug).then(|| LogMessage::new(&message(), LogLevel::Debug, None))
}

#[inline]
fn finish_debug_log(ctx: &mut ExecutionContext, log: &mut Option<LogMessage>) {
    if let Some(mut log) = log.take() {
        log.end();
        ctx.log(log);
    }
}

fn push_execution_target(
    targets: &mut Vec<ExecutionTarget>,
    node: Arc<InternalNode>,
    through_pin: Arc<InternalPin>,
) {
    if let Some(target) = targets
        .iter_mut()
        .find(|target| Arc::ptr_eq(&target.node, &node))
    {
        target.through_pins.push(through_pin);
        return;
    }

    targets.push(ExecutionTarget {
        node,
        through_pins: vec![through_pin],
    });
}

fn visit_execution_connection(
    pin: Arc<InternalPin>,
    targets: &mut Vec<ExecutionTarget>,
    visited: &mut InlineVisitedPins<16>,
    relay_stack: &mut Vec<Weak<InternalPin>>,
) {
    if !visited.insert(&pin) {
        return;
    }

    if let Some(node) = pin.node().and_then(Weak::upgrade) {
        push_execution_target(targets, node, pin);
    } else {
        relay_stack.extend(pin.connected_to().iter().cloned());
    }
}

/// The common case has one active execution output and direct connections. It needs only
/// the returned target vectors; relay traversal and cycle tracking stay allocation-free for
/// short chains.
fn targets_for_single_exec_output(pin: &Arc<InternalPin>) -> Vec<ExecutionTarget> {
    let connections = pin.connected_to();
    let mut targets = Vec::with_capacity(connections.len());
    let mut visited = InlineVisitedPins::<16>::new();
    let mut relay_stack = Vec::new();

    for connection in connections {
        if let Some(pin) = connection.upgrade() {
            visit_execution_connection(pin, &mut targets, &mut visited, &mut relay_stack);
        }
    }

    while let Some(connection) = relay_stack.pop() {
        if let Some(pin) = connection.upgrade() {
            visit_execution_connection(pin, &mut targets, &mut visited, &mut relay_stack);
        }
    }

    targets
}

async fn exec_deps_from_map(
    ctx: &mut ExecutionContext,
    recursion_guard: &mut Option<AHashSet<String>>,
    dependencies: &AHashMap<String, Vec<Arc<InternalNode>>>,
) -> bool {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Phase {
        Enter,
        Exit,
    }

    let mut stack: Vec<(Arc<InternalNode>, Phase)> = Vec::new();
    if let Some(roots) = dependencies.get(ctx.node.node_id()) {
        stack.reserve(roots.len().saturating_mul(2));
        for dep in roots.iter() {
            stack.push((dep.clone(), Phase::Enter));
        }
    }

    let mut scheduled: AHashSet<usize> = AHashSet::with_capacity(stack.len().saturating_mul(2));
    let mut visiting: AHashSet<usize> = AHashSet::with_capacity(stack.len().saturating_mul(2));

    while let Some((n, phase)) = stack.pop() {
        let n_ptr = ptr_key(&n);

        match phase {
            Phase::Enter => {
                if scheduled.contains(&n_ptr) {
                    continue;
                }
                if !visiting.insert(n_ptr) {
                    ctx.log_message(
                        "Cycle detected while resolving mapped dependencies",
                        LogLevel::Error,
                    );
                    return false;
                }
                stack.push((n.clone(), Phase::Exit));

                if let Some(children) = dependencies.get(n.node_id()) {
                    for c in children.iter() {
                        let c_ptr = ptr_key(c);
                        if scheduled.contains(&c_ptr) {
                            continue;
                        }
                        stack.push((c.clone(), Phase::Enter));
                    }
                }
            }
            Phase::Exit => {
                visiting.remove(&n_ptr);
                if scheduled.contains(&n_ptr) {
                    continue;
                }

                if let Some(guard) = recursion_guard
                    && guard.contains(n.node_id())
                {
                    log_debug(ctx, || {
                        format!(
                            "Recursion detected for: {}, skipping execution",
                            n.node_id()
                        )
                    });
                    scheduled.insert(n_ptr);
                    continue;
                }

                let mut sub = ctx.create_sub_context(&n).await;
                let mut log_message = start_debug_log(ctx, || {
                    format!("Triggering mapped dependency: {}", n.node_name())
                });

                // Reuse your non-recursive single-node runner
                let res = run_node_logic_only(&mut sub, recursion_guard).await;

                finish_debug_log(ctx, &mut log_message);
                sub.end_trace();
                ctx.push_sub_context(&mut sub);

                if res.is_err() {
                    ctx.log_message("Failed to trigger mapped dependency", LogLevel::Error);
                    return false;
                }

                scheduled.insert(n_ptr);
            }
        }
    }

    true
}

async fn run_node_logic_only(
    ctx: &mut ExecutionContext,
    recursion_guard: &mut Option<AHashSet<String>>,
) -> flow_like_types::Result<(), InternalNodeError> {
    // Check for cancellation before starting
    if ctx.is_cancelled() {
        ctx.log_message("Execution cancelled before node started", LogLevel::Warn);
        ctx.set_state(NodeState::Error).await;
        return Err(InternalNodeError::ExecutionFailed("Cancelled".to_string()));
    }

    ctx.set_state(NodeState::Running).await;
    let node = ctx.node.clone();

    if recursion_guard.is_none() {
        *recursion_guard = Some(AHashSet::new());
    }
    if let Some(guard) = recursion_guard {
        if guard.contains(node.node_id()) {
            log_debug(ctx, || {
                format!("Recursion detected for: {}", node.node_id())
            });
            ctx.end_trace();
            return Ok(());
        }
        guard.insert(node.node_id().to_string());
    }

    let logic = node.logic.clone();
    let mut log_message = start_debug_log(ctx, || {
        format!(
            "Starting Node Execution: {} [{}]",
            node.node_name(),
            node.node_id()
        )
    });

    ctx.increment_nodes_executed();
    let result = logic.run(ctx).await;

    // Check for cancellation after node execution
    if ctx.is_cancelled() {
        ctx.log_message("Execution cancelled after node completed", LogLevel::Warn);
        finish_debug_log(ctx, &mut log_message);
        ctx.end_trace();
        ctx.set_state(NodeState::Error).await;
        return Err(InternalNodeError::ExecutionFailed("Cancelled".to_string()));
    }

    if let Err(e) = result {
        let err_string = format!("{:?}", e);
        ctx.log_message(
            &format!("Failed to execute node: {}", &err_string),
            LogLevel::Error,
        );
        finish_debug_log(ctx, &mut log_message);
        ctx.end_trace();
        ctx.set_state(NodeState::Error).await;
        return Err(InternalNodeError::ExecutionFailed(err_string));
    }

    ctx.set_state(NodeState::Success).await;
    finish_debug_log(ctx, &mut log_message);
    ctx.end_trace();
    Ok(())
}

// --- Helper: collect *pure* parent nodes for a given InternalNode ------------------
async fn pure_parents_for_memo(
    node: &Arc<InternalNode>,
    memo: &mut AHashMap<usize, Vec<Arc<InternalNode>>>,
) -> flow_like_types::Result<Vec<Arc<InternalNode>>> {
    let key = ptr_key(node);
    if let Some(v) = memo.get(&key) {
        return Ok(v.clone());
    }

    let mut result: Vec<Arc<InternalNode>> = Vec::new();

    // Iterate only input, non-exec pins. Relay through standalone pins.
    for pin in node.pins.values() {
        // Direct access to immutable fields - no lock needed
        let is_input = pin.pin_type == PinType::Input;
        let is_exec = pin.data_type == VariableType::Execution;

        if !is_input || is_exec {
            continue;
        }

        // Pointer-keyed visited for pins; reserve generously to avoid rehash
        let deps = pin.depends_on();
        let depends_on_len = deps.len();
        let mut visited_pins: AHashSet<usize> =
            AHashSet::with_capacity(depends_on_len.saturating_mul(4));
        let mut stack: Vec<Weak<InternalPin>> = deps.to_vec();

        while let Some(dep_weak) = stack.pop() {
            let Some(dep_arc) = dep_weak.upgrade() else {
                continue;
            };
            let pin_key = ptr_key(&dep_arc);
            if !visited_pins.insert(pin_key) {
                continue;
            }

            // Direct access - no lock needed
            if let Some(node_weak) = dep_arc.node() {
                if let Some(parent) = node_weak.upgrade()
                    && parent.is_pure().await
                {
                    result.push(parent);
                }
            } else {
                // standalone/relay pin => follow further upstream
                let dep_deps = dep_arc.depends_on();
                if !dep_deps.is_empty() {
                    stack.extend(dep_deps.iter().cloned());
                }
            }
        }
    }

    // (Optional) de-dup parents by pointer to avoid re-executing same pure node.
    if result.len() > 1 {
        let mut seen: AHashSet<usize> = AHashSet::with_capacity(result.len());
        result.retain(|n| seen.insert(ptr_key(n)));
    }

    memo.insert(key, result.clone());
    Ok(result)
}

/// Cached immutable metadata from Node to avoid locking
#[derive(Clone)]
pub struct NodeMeta {
    pub id: Arc<str>,
    pub name: String,
    pub is_pure: bool,
}

impl NodeMeta {
    pub fn from_node(node: &Node) -> Self {
        Self {
            id: Arc::from(node.id.as_str()),
            name: node.name.clone(),
            is_pure: node.is_pure(),
        }
    }
}

pub struct InternalNode {
    pub node: Arc<Mutex<Node>>,
    /// Cached immutable metadata - no lock needed for access
    pub meta: NodeMeta,
    pub pins: AHashMap<String, Arc<InternalPin>>,
    pub logic: Arc<dyn NodeLogic>,
    pub exec_calls: AtomicU64,
    pin_name_cache: AHashMap<String, Vec<Arc<InternalPin>>>,
}

impl InternalNode {
    pub fn new(
        node: Node,
        pins: AHashMap<String, Arc<InternalPin>>,
        logic: Arc<dyn NodeLogic>,
        mut name_cache: AHashMap<String, Vec<Arc<InternalPin>>>,
    ) -> Self {
        let meta = NodeMeta::from_node(&node);

        // Preserve the caller's duplicate-name ordering, but make the eager index complete and
        // self-consistent at the construction boundary.
        let mut indexed_pins = AHashSet::with_capacity(pins.len());
        name_cache.retain(|name, cached| {
            cached.retain(|pin| {
                pin.name() == name
                    && pins
                        .get(pin.id())
                        .is_some_and(|entry| Arc::ptr_eq(entry, pin))
                    && indexed_pins.insert(ptr_key(pin))
            });
            !cached.is_empty()
        });
        for pin in pins.values() {
            let is_cached = name_cache
                .get(pin.name())
                .is_some_and(|cached| cached.iter().any(|entry| Arc::ptr_eq(entry, pin)));
            if !is_cached {
                name_cache
                    .entry(pin.name().to_string())
                    .or_default()
                    .push(pin.clone());
            }
        }

        debug_assert_eq!(
            name_cache.values().map(Vec::len).sum::<usize>(),
            pins.len(),
            "pin name cache must contain every pin exactly once"
        );
        debug_assert!(
            pins.values().all(|pin| {
                name_cache
                    .get(pin.name())
                    .is_some_and(|cached| cached.iter().any(|entry| Arc::ptr_eq(entry, pin)))
            }),
            "pin name cache must cover every pin under its name"
        );
        debug_assert!(
            name_cache.iter().all(|(name, cached)| {
                cached.iter().all(|pin| {
                    pin.name() == name
                        && pins
                            .get(pin.id())
                            .is_some_and(|entry| Arc::ptr_eq(entry, pin))
                })
            }),
            "pin name cache must not contain stale or misnamed pins"
        );

        InternalNode {
            node: Arc::new(Mutex::new(node)),
            meta,
            pins,
            logic,
            pin_name_cache: name_cache,
            exec_calls: AtomicU64::new(0),
        }
    }

    /// Get cached node ID without locking
    #[inline]
    pub fn node_id(&self) -> &str {
        self.meta.id.as_ref()
    }

    /// Clone the cached node ID without allocating.
    #[inline]
    pub fn shared_node_id(&self) -> Arc<str> {
        self.meta.id.clone()
    }

    /// Get cached node name without locking
    #[inline]
    pub fn node_name(&self) -> &str {
        &self.meta.name
    }

    /// Get cached is_pure without locking
    #[inline]
    pub fn is_pure_cached(&self) -> bool {
        self.meta.is_pure
    }

    /// Retained for source compatibility; the immutable cache is populated eagerly by `new`.
    #[inline]
    pub async fn ensure_cache(&self, _name: &str) {}

    pub async fn get_pin_by_name(&self, name: &str) -> flow_like_types::Result<Arc<InternalPin>> {
        self.pin_name_cache
            .get(name)
            .and_then(|pins| pins.first())
            .cloned()
            .ok_or_else(|| flow_like_types::anyhow!("Pin {} not found", name))
    }

    pub async fn get_pins_by_name(
        &self,
        name: &str,
    ) -> flow_like_types::Result<Vec<Arc<InternalPin>>> {
        self.pin_name_cache
            .get(name)
            .cloned()
            .ok_or_else(|| flow_like_types::anyhow!("Pin {} not found", name))
    }

    pub fn get_pin_by_id(&self, id: &str) -> flow_like_types::Result<Arc<InternalPin>> {
        if let Some(pin) = self.pins.get(id) {
            return Ok(pin.clone());
        }

        Err(flow_like_types::anyhow!("Pin {} not found", id))
    }

    pub async fn orphaned(&self) -> bool {
        for pin in self.pins.values() {
            // No lock needed - direct access to immutable fields
            if pin.pin_type != PinType::Input {
                continue;
            }

            if pin.depends_on().is_empty() && pin.default_value.is_none() {
                return true;
            }
        }

        false
    }

    pub async fn is_ready(&self) -> flow_like_types::Result<bool> {
        for pin in self.pins.values() {
            // Direct access to immutable fields - no lock needed
            let pin_type = &pin.pin_type;
            let data_type = &pin.data_type;

            if *pin_type != PinType::Input {
                continue;
            }

            let has_default = pin.has_default;
            let deps = pin.depends_on();

            if deps.is_empty() && !has_default {
                return Ok(false);
            }

            // execution pins can have multiple inputs for different paths leading to it. We only need to make sure that one of them is valid!
            let is_execution = *data_type == VariableType::Execution;
            let mut execution_valid = false;

            for depends_on_pin in deps {
                let depends_on_pin = depends_on_pin
                    .upgrade()
                    .ok_or_else(|| flow_like_types::anyhow!("Failed to lock Pin"))?;

                // Only value access needs locking
                let has_value = depends_on_pin.value.read().is_some();

                // non execution pins need all inputs to be valid
                if !has_value && !is_execution {
                    return Ok(false);
                }

                if has_value {
                    execution_valid = true;
                }
            }

            if is_execution && !execution_valid {
                return Ok(false);
            }
        }

        Ok(true)
    }

    pub async fn get_connected(&self) -> flow_like_types::Result<Vec<Arc<InternalNode>>> {
        let mut connected = Vec::with_capacity(self.pins.len());
        let mut seen_nodes: AHashSet<usize> = AHashSet::new();
        let mut visited_pins: AHashSet<usize> = AHashSet::new();
        let mut stack: Vec<Weak<InternalPin>> = Vec::new();

        for pin in self.pins.values() {
            // Direct access to immutable fields - no lock needed
            if pin.pin_type != PinType::Output {
                continue;
            }

            let conn = pin.connected_to();
            let cap = conn.len();
            visited_pins.clear();
            stack.clear();
            if stack.capacity() < cap {
                stack.reserve(cap - stack.capacity());
            }
            stack.extend(conn.iter().cloned());

            while let Some(next_weak) = stack.pop() {
                let pin_arc = next_weak
                    .upgrade()
                    .ok_or_else(|| flow_like_types::anyhow!("Failed to lock Pin"))?;

                let pin_key = Arc::as_ptr(&pin_arc) as usize;
                if !visited_pins.insert(pin_key) {
                    continue;
                }

                // Direct access - no lock needed
                if let Some(node_weak) = pin_arc.node() {
                    if let Some(parent) = node_weak.upgrade() {
                        let node_key = Arc::as_ptr(&parent) as usize;
                        if seen_nodes.insert(node_key) {
                            connected.push(parent);
                        }
                    }
                } else {
                    stack.extend(pin_arc.connected_to().iter().cloned());
                }
            }
        }

        Ok(connected)
    }

    pub async fn get_connected_exec(
        &self,
        filter_valid: bool,
        context: &ExecutionContext,
    ) -> flow_like_types::Result<Vec<ExecutionTarget>> {
        let mut first_active_output = None;
        let mut additional_active_outputs = Vec::new();

        for pin in self.pins.values() {
            // Direct access to immutable fields - no lock needed
            if pin.pin_type != PinType::Output || pin.data_type != VariableType::Execution {
                continue;
            }

            if filter_valid {
                match evaluate_pin_value(pin.clone(), &context.context_pin_overrides).await {
                    Ok(Value::Bool(true)) => {}
                    _ => continue,
                }
            }

            if first_active_output.is_none() {
                first_active_output = Some(pin.clone());
            } else {
                additional_active_outputs.push(pin.clone());
            }
        }

        let Some(first_active_output) = first_active_output else {
            return Ok(Vec::new());
        };

        if additional_active_outputs.is_empty() {
            return Ok(targets_for_single_exec_output(&first_active_output));
        }

        // node_ptr -> (node_arc, pins_vec, seen_pin_ptrs)
        let mut groups: AHashMap<
            usize,
            (Arc<InternalNode>, Vec<Arc<InternalPin>>, AHashSet<usize>),
        > = AHashMap::with_capacity(
            first_active_output
                .connected_to()
                .len()
                .saturating_add(additional_active_outputs.len()),
        );

        let mut visited_pins: AHashSet<usize> = AHashSet::with_capacity(64);
        let mut stack: Vec<Weak<InternalPin>> = Vec::with_capacity(64);

        for pin in std::iter::once(first_active_output).chain(additional_active_outputs) {
            visited_pins.clear();
            stack.clear();
            stack.extend(pin.connected_to().iter().cloned());

            while let Some(next_weak) = stack.pop() {
                let Some(pin_arc) = next_weak.upgrade() else {
                    continue;
                };
                let pkey = ptr_key(&pin_arc);
                if !visited_pins.insert(pkey) {
                    continue;
                }

                // Direct access - no lock needed
                if let Some(node_w) = pin_arc.node() {
                    if let Some(parent) = node_w.upgrade() {
                        let nkey = ptr_key(&parent);
                        let entry = groups.entry(nkey).or_insert_with(|| {
                            (
                                parent.clone(),
                                Vec::with_capacity(2),
                                AHashSet::with_capacity(4),
                            )
                        });
                        // dedup pin within the node group
                        if entry.2.insert(pkey) {
                            entry.1.push(pin_arc.clone());
                        }
                    }
                } else {
                    // relay pin; keep walking
                    stack.extend(pin_arc.connected_to().iter().cloned());
                }
            }
        }

        // materialize
        let mut out = Vec::with_capacity(groups.len());
        for (_, (node, pins, _seen)) in groups {
            out.push(ExecutionTarget {
                node,
                through_pins: pins,
            });
        }
        Ok(out)
    }

    pub async fn get_error_handled_nodes(
        &self,
        context: &ExecutionContext,
    ) -> flow_like_types::Result<Vec<Arc<InternalNode>>> {
        let pin = self.get_pin_by_name("auto_handle_error").await?;
        let active = evaluate_pin_value(pin.clone(), &context.context_pin_overrides).await?;
        let active = match active {
            Value::Bool(b) => b,
            _ => false,
        };
        if !active {
            return Err(flow_like_types::anyhow!("Error Pin not active"));
        }

        // Direct access to immutable fields - no lock needed
        if pin.pin_type != PinType::Output {
            return Err(flow_like_types::anyhow!("Pin is not an output pin"));
        }
        if pin.data_type != VariableType::Execution {
            return Err(flow_like_types::anyhow!("Pin is not an execution pin"));
        }

        let conn = pin.connected_to();
        let cap = conn.len();
        let mut connected = Vec::with_capacity(cap);
        let mut seen_nodes: AHashSet<usize> = AHashSet::with_capacity(cap.saturating_mul(2));
        let mut visited_pins: AHashSet<usize> = AHashSet::with_capacity(cap.saturating_mul(4));
        let mut stack: Vec<Weak<InternalPin>> = conn.to_vec();

        while let Some(next_weak) = stack.pop() {
            let pin_arc = next_weak
                .upgrade()
                .ok_or_else(|| flow_like_types::anyhow!("Failed to lock Pin"))?;

            let pin_key = Arc::as_ptr(&pin_arc) as usize;
            if !visited_pins.insert(pin_key) {
                continue;
            }

            // Direct access - no lock needed
            if let Some(node_weak) = pin_arc.node() {
                if let Some(parent) = node_weak.upgrade() {
                    let node_key = Arc::as_ptr(&parent) as usize;
                    if seen_nodes.insert(node_key) {
                        connected.push(parent);
                    }
                }
            } else {
                // relay through standalone pins
                stack.extend(pin_arc.connected_to().iter().cloned());
            }
        }

        Ok(connected)
    }

    pub async fn get_dependencies(&self) -> flow_like_types::Result<Vec<Arc<InternalNode>>> {
        let mut dependencies = Vec::with_capacity(self.pins.len());
        let mut seen_nodes: AHashSet<usize> = AHashSet::new();
        let mut visited_pins: AHashSet<usize> = AHashSet::new();
        let mut stack: Vec<Weak<InternalPin>> = Vec::new();

        for pin in self.pins.values() {
            // Direct access to immutable fields - no lock needed
            if pin.pin_type != PinType::Input {
                continue;
            }

            let deps = pin.depends_on();
            let cap = deps.len();
            visited_pins.clear();
            stack.clear();
            if stack.capacity() < cap {
                stack.reserve(cap - stack.capacity());
            }
            stack.extend(deps.iter().cloned());

            while let Some(dep_weak) = stack.pop() {
                let dep_arc = dep_weak
                    .upgrade()
                    .ok_or_else(|| flow_like_types::anyhow!("Failed to lock Pin"))?;

                let pin_key = Arc::as_ptr(&dep_arc) as usize;
                if !visited_pins.insert(pin_key) {
                    continue;
                }

                // Direct access - no lock needed
                if let Some(node_weak) = dep_arc.node() {
                    if let Some(parent) = node_weak.upgrade() {
                        let node_key = Arc::as_ptr(&parent) as usize;
                        if seen_nodes.insert(node_key) {
                            dependencies.push(parent);
                        }
                    }
                } else {
                    stack.extend(dep_arc.depends_on().iter().cloned());
                }
            }
        }

        Ok(dependencies)
    }

    /// Use cached is_pure for performance
    pub async fn is_pure(&self) -> bool {
        self.meta.is_pure
    }

    pub async fn trigger_missing_dependencies(
        context: &mut ExecutionContext,
        recursion_guard: &mut Option<AHashSet<String>>,
        _with_successors: bool, // not used here
    ) -> bool {
        #[derive(Clone, Copy, PartialEq, Eq)]
        enum Phase {
            Enter,
            Exit,
        }

        let mut parents_memo: AHashMap<usize, Vec<Arc<InternalNode>>> = AHashMap::with_capacity(16);

        // Seed: pure parents of the current node. Dedup by pointer.
        let mut roots = match pure_parents_for_memo(&context.node, &mut parents_memo).await {
            Ok(v) => v,
            Err(_) => {
                context.log_message("Failed to collect dependencies", LogLevel::Error);
                return false;
            }
        };
        if roots.len() > 1 {
            let mut seen: AHashSet<usize> = AHashSet::with_capacity(roots.len());
            roots.retain(|n| seen.insert(ptr_key(n)));
        }

        let mut stack: Vec<(Arc<InternalNode>, Phase)> =
            Vec::with_capacity(roots.len().saturating_mul(2));
        for n in roots {
            stack.push((n, Phase::Enter));
        }

        let mut scheduled: AHashSet<usize> = AHashSet::with_capacity(stack.len().saturating_mul(2));
        let mut visiting: AHashSet<usize> = AHashSet::with_capacity(stack.len().saturating_mul(2));

        while let Some((node_arc, phase)) = stack.pop() {
            let node_ptr = ptr_key(&node_arc);

            match phase {
                Phase::Enter => {
                    if scheduled.contains(&node_ptr) {
                        continue;
                    }
                    if !visiting.insert(node_ptr) {
                        context.log_message(
                            "Cycle detected while resolving dependencies",
                            LogLevel::Error,
                        );
                        return false;
                    }

                    // Post-order: revisit on Exit
                    stack.push((node_arc.clone(), Phase::Exit));

                    // Push this node's pure parents first (dedup against scheduled for cheap)
                    match pure_parents_for_memo(&node_arc, &mut parents_memo).await {
                        Ok(parents) => {
                            // Iterate parents in natural order; for more cache locality you could reverse.
                            for p in parents {
                                let p_ptr = ptr_key(&p);
                                if scheduled.contains(&p_ptr) {
                                    continue;
                                }
                                stack.push((p, Phase::Enter));
                            }
                        }
                        Err(e) => {
                            context.log_message(
                                &format!("Failed to collect parents: {:?}", e),
                                LogLevel::Error,
                            );
                            return false;
                        }
                    }
                }
                Phase::Exit => {
                    visiting.remove(&node_ptr);
                    if scheduled.contains(&node_ptr) {
                        continue;
                    }

                    if let Some(guard) = recursion_guard
                        && guard.contains(node_arc.node_id())
                    {
                        log_debug(context, || {
                            format!(
                                "Recursion detected for: {}, skipping execution",
                                node_arc.node_id()
                            )
                        });
                        scheduled.insert(node_ptr);
                        continue;
                    }

                    // Execute dependency (no successors)
                    let mut sub = context.create_sub_context(&node_arc).await;
                    let mut log_message = start_debug_log(context, || {
                        format!("Triggering missing dependency: {}", node_arc.node_name())
                    });
                    let res = run_node_logic_only(&mut sub, recursion_guard).await;
                    finish_debug_log(context, &mut log_message);
                    sub.end_trace();
                    context.push_sub_context(&mut sub);

                    if res.is_err() {
                        context.log_message(
                            &format!("Failed to trigger dependency: {}", node_arc.node_name()),
                            LogLevel::Error,
                        );
                        return false;
                    }

                    scheduled.insert(node_ptr);
                }
            }
        }

        true
    }

    pub async fn handle_error(
        context: &mut ExecutionContext,
        error: &str,
        recursion_guard: &mut Option<AHashSet<String>>,
    ) -> Result<(), InternalNodeError> {
        let _ = context.activate_exec_pin("auto_handle_error").await;
        let _ = context
            .set_pin_value("auto_handle_error_string", json!(error))
            .await;

        let connected = context
            .node
            .get_error_handled_nodes(context)
            .await
            .map_err(|err| {
                let err_string = format!("Failed to get error handling nodes: {}", err);
                context.log_message(&err_string, LogLevel::Error);
                InternalNodeError::ExecutionFailed(error.to_string())
            })?;

        if connected.is_empty() {
            context.log_message(
                &format!("No error handling nodes found for: {}", &context.id),
                LogLevel::Error,
            );
            return Err(InternalNodeError::ExecutionFailed(error.to_string()));
        }

        // Iterate each error handler and walk its successors iteratively (DFS).
        for handler in connected {
            let mut sub = context.create_sub_context(&handler).await;

            // Use SAME recursion_guard here (parity with original)
            if !InternalNode::trigger_missing_dependencies(&mut sub, recursion_guard, false).await {
                let err_string =
                    "Failed to trigger missing dependencies for error handler".to_string();
                let _ = sub
                    .set_pin_value("auto_handle_error_string", json!(err_string.clone()))
                    .await;
                sub.end_trace();
                context.push_sub_context(&mut sub);
                return Err(InternalNodeError::ExecutionFailed(err_string));
            }

            // run handler node
            if let Err(e) = run_node_logic_only(&mut sub, recursion_guard).await {
                let err_string = e.into_message();
                let _ = sub
                    .set_pin_value("auto_handle_error_string", json!(err_string.clone()))
                    .await;
                sub.end_trace();
                context.push_sub_context(&mut sub);
                return Err(InternalNodeError::ExecutionFailed(err_string));
            }

            // walk successors of the error handler (still using the same guard)
            let mut stack: Vec<ExecutionTarget> =
                match handler.get_connected_exec(true, context).await {
                    Ok(v) => v,
                    Err(err) => {
                        let err_string = format!("{:?}", err);
                        let _ = sub
                            .set_pin_value("auto_handle_error_string", json!(err_string.clone()))
                            .await;
                        sub.end_trace();
                        context.push_sub_context(&mut sub);
                        return Err(InternalNodeError::ExecutionFailed(err_string));
                    }
                };

            let mut seen_exec_ptrs: ahash::AHashSet<usize> =
                ahash::AHashSet::with_capacity(stack.len().saturating_mul(2));

            while let Some(next) = stack.pop() {
                let key = Arc::as_ptr(&next.node) as usize;
                if !seen_exec_ptrs.insert(key) {
                    continue;
                }

                let mut sub2 = next.into_sub_context(context).await;

                if !InternalNode::trigger_missing_dependencies(&mut sub2, recursion_guard, false)
                    .await
                {
                    let err_string =
                        "Failed to trigger successor dependencies (error chain)".to_string();
                    let _ = sub2
                        .set_pin_value("auto_handle_error_string", json!(err_string.clone()))
                        .await;
                    sub2.end_trace();
                    context.push_sub_context(&mut sub2);
                    let _ = sub
                        .set_pin_value("auto_handle_error_string", json!("error chain aborted"))
                        .await;
                    sub.end_trace();
                    context.push_sub_context(&mut sub);
                    return Err(InternalNodeError::ExecutionFailed(err_string));
                }

                if let Err(e) = run_node_logic_only(&mut sub2, recursion_guard).await {
                    let err_string = e.into_message();
                    let _ = sub2
                        .set_pin_value("auto_handle_error_string", json!(err_string.clone()))
                        .await;
                    sub2.end_trace();
                    context.push_sub_context(&mut sub2);
                    let _ = sub
                        .set_pin_value("auto_handle_error_string", json!("error chain aborted"))
                        .await;
                    sub.end_trace();
                    context.push_sub_context(&mut sub);
                    return Err(InternalNodeError::ExecutionFailed(err_string));
                }

                match next.node.get_connected_exec(true, context).await {
                    Ok(more) => {
                        for s in more {
                            stack.push(s);
                        }
                    }
                    Err(err) => {
                        let err_string = format!("{:?}", err);
                        let _ = sub2
                            .set_pin_value("auto_handle_error_string", json!(err_string.clone()))
                            .await;
                        sub2.end_trace();
                        context.push_sub_context(&mut sub2);
                        let _ = sub
                            .set_pin_value("auto_handle_error_string", json!("error chain aborted"))
                            .await;
                        sub.end_trace();
                        context.push_sub_context(&mut sub);
                        return Err(InternalNodeError::ExecutionFailed(err_string));
                    }
                }

                sub2.end_trace();
                context.push_sub_context(&mut sub2);
            }

            sub.end_trace();
            context.push_sub_context(&mut sub);
        }

        context.set_state(NodeState::Error).await;
        Ok(())
    }

    pub async fn trigger(
        context: &mut ExecutionContext,
        recursion_guard: &mut Option<AHashSet<String>>,
        with_successors: bool,
    ) -> flow_like_types::Result<(), InternalNodeError> {
        // deps
        if !InternalNode::trigger_missing_dependencies(context, recursion_guard, false).await {
            context.log_message("Failed to trigger missing dependencies", LogLevel::Error);
            context.end_trace();
            InternalNode::handle_error(
                context,
                "Failed to trigger missing dependencies",
                recursion_guard,
            )
            .await?;
            return Err(InternalNodeError::DependencyFailed(
                context.node.node_id().to_string(),
            ));
        }

        // this node
        if let Err(e) = run_node_logic_only(context, recursion_guard).await {
            let err_string = e.into_message();
            context.log_message(
                &format!("Failed to execute node: {}", err_string),
                LogLevel::Error,
            );
            InternalNode::handle_error(context, &err_string, recursion_guard).await?;
            return Err(InternalNodeError::ExecutionFailed(err_string));
        }

        // successors (DFS; fresh guard per successor to mirror old semantics)
        if with_successors {
            let successors = match context.node.get_connected_exec(true, context).await {
                Ok(nodes) => nodes,
                Err(err) => {
                    let err_string = format!("{:?}", err);
                    context.log_message(
                        &format!("Failed to get successors: {}", err_string),
                        LogLevel::Error,
                    );
                    InternalNode::handle_error(context, &err_string, recursion_guard).await?;
                    return Err(InternalNodeError::ExecutionFailed(err_string));
                }
            };

            let mut stack: Vec<ExecutionTarget> = Vec::with_capacity(successors.len());
            stack.extend(successors);

            let mut seen_exec_ptrs: ahash::AHashSet<usize> =
                ahash::AHashSet::with_capacity(stack.len().saturating_mul(2));

            while let Some(next) = stack.pop() {
                let key = Arc::as_ptr(&next.node) as usize;
                if !seen_exec_ptrs.insert(key) {
                    continue;
                }

                let mut sub = next.into_sub_context(context).await;
                let mut local_guard: Option<AHashSet<String>> = None;

                if !InternalNode::trigger_missing_dependencies(&mut sub, &mut local_guard, false)
                    .await
                {
                    let err_string = "Failed to trigger successor dependencies".to_string();
                    InternalNode::handle_error(&mut sub, &err_string, &mut local_guard).await?;
                    sub.end_trace();
                    context.push_sub_context(&mut sub);
                    return Err(InternalNodeError::ExecutionFailed(err_string));
                }

                if let Err(e) = run_node_logic_only(&mut sub, &mut local_guard).await {
                    let err_string = e.into_message();
                    let _ = sub.activate_exec_pin("auto_handle_error").await;
                    let _ = sub
                        .set_pin_value("auto_handle_error_string", json!(err_string.clone()))
                        .await;
                    sub.end_trace();
                    context.push_sub_context(&mut sub);
                    return Err(InternalNodeError::ExecutionFailed(err_string));
                }

                match next.node.get_connected_exec(true, context).await {
                    Ok(more) => {
                        for s in more {
                            stack.push(s);
                        }
                    }
                    Err(err) => {
                        let err_string = format!("{:?}", err);
                        InternalNode::handle_error(&mut sub, &err_string, &mut local_guard).await?;
                        sub.end_trace();
                        context.push_sub_context(&mut sub);
                        return Err(InternalNodeError::ExecutionFailed(err_string));
                    }
                }

                sub.end_trace();
                context.push_sub_context(&mut sub);
            }
        }

        Ok(())
    }

    pub async fn trigger_with_dependencies(
        context: &mut ExecutionContext,
        recursion_guard: &mut Option<AHashSet<String>>,
        with_successors: bool,
        dependencies: &AHashMap<String, Vec<Arc<InternalNode>>>,
    ) -> flow_like_types::Result<(), InternalNodeError> {
        context.set_state(NodeState::Running).await;

        let node = context.node.clone();

        if recursion_guard.is_none() {
            *recursion_guard = Some(AHashSet::new());
        }
        if let Some(guard) = recursion_guard {
            if guard.contains(node.node_id()) {
                log_debug(context, || {
                    format!("Recursion detected for: {}", node.node_id())
                });
                context.end_trace();
                return Ok(());
            }
            guard.insert(node.node_id().to_string());
        }

        // 1) Execute precomputed dependencies iteratively (no recursion)
        if !exec_deps_from_map(context, recursion_guard, dependencies).await {
            let err = "Failed to trigger mapped dependencies".to_string();
            InternalNode::handle_error(context, &err, recursion_guard).await?;
            return Err(InternalNodeError::DependencyFailed(
                node.node_id().to_string(),
            ));
        }

        // 2) Run this node (no successors here)
        let logic = node.logic.clone();
        let mut log_message = start_debug_log(context, || {
            format!(
                "Starting Node Execution: {} [{}]",
                node.node_name(),
                node.node_id()
            )
        });
        context.increment_nodes_executed();
        let result = logic.run(context).await;

        if let Err(e) = result {
            let err_string = format!("{:?}", e);
            context.log_message(
                &format!("Failed to execute node: {}", err_string),
                LogLevel::Error,
            );
            finish_debug_log(context, &mut log_message);
            context.end_trace();
            context.set_state(NodeState::Error).await;
            InternalNode::handle_error(context, &err_string, recursion_guard).await?;
            return Err(InternalNodeError::ExecutionFailed(err_string));
        }

        context.set_state(NodeState::Success).await;
        finish_debug_log(context, &mut log_message);
        context.end_trace();

        // 3) Walk successors iteratively (DFS), like your non-recursive `trigger`
        if with_successors {
            let successors = match context.node.get_connected_exec(true, context).await {
                Ok(nodes) => nodes,
                Err(err) => {
                    let err_string = format!("{:?}", err);
                    context.log_message(
                        &format!("Failed to get successors: {}", err_string.clone()),
                        LogLevel::Error,
                    );
                    InternalNode::handle_error(context, &err_string, recursion_guard).await?;
                    return Err(InternalNodeError::ExecutionFailed(err_string));
                }
            };

            let mut stack: Vec<ExecutionTarget> = Vec::with_capacity(successors.len());
            stack.extend(successors);

            let mut seen_exec_ptrs: ahash::AHashSet<usize> =
                ahash::AHashSet::with_capacity(stack.len().saturating_mul(2));

            while let Some(next) = stack.pop() {
                let key = Arc::as_ptr(&next.node) as usize;
                if !seen_exec_ptrs.insert(key) {
                    continue;
                }

                let mut sub = next.into_sub_context(context).await;

                // Fresh recursion guard per successor to mirror original semantics
                let mut local_guard: Option<AHashSet<String>> = None;

                // Execute *its* mapped deps (fresh executed set semantics like before)
                if !exec_deps_from_map(&mut sub, &mut local_guard, dependencies).await {
                    let err_string = "Failed to trigger successor mapped dependencies".to_string();
                    InternalNode::handle_error(&mut sub, &err_string, &mut local_guard).await?;
                    sub.end_trace();
                    context.push_sub_context(&mut sub);
                    return Err(InternalNodeError::ExecutionFailed(err_string));
                }

                // Run successor node
                if let Err(e) = run_node_logic_only(&mut sub, &mut local_guard).await {
                    let err_string = e.into_message();
                    let _ = sub.activate_exec_pin("auto_handle_error").await;
                    let _ = sub
                        .set_pin_value("auto_handle_error_string", json!(err_string.clone()))
                        .await;
                    sub.end_trace();
                    context.push_sub_context(&mut sub);
                    return Err(InternalNodeError::ExecutionFailed(err_string));
                }

                // Enqueue its successors (DFS)
                match next.node.get_connected_exec(true, context).await {
                    Ok(more) => {
                        for s in more {
                            stack.push(s);
                        }
                    }
                    Err(err) => {
                        let err_string = format!("{:?}", err);
                        InternalNode::handle_error(&mut sub, &err_string, &mut local_guard).await?;
                        sub.end_trace();
                        context.push_sub_context(&mut sub);
                        return Err(InternalNodeError::ExecutionFailed(err_string));
                    }
                }

                sub.end_trace();
                context.push_sub_context(&mut sub);
            }
        }

        Ok(())
    }
}
