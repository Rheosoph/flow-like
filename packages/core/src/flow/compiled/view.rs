//! Board view reconstruction from a compiled artifact.
//!
//! Runtime consumers (`context.get_board()`, `read_node()`, call_function's
//! layer lookups, agent tool generation) receive this view instead of the
//! editor board. Every execution-relevant field round-trips; editor-only
//! fields (coordinates, comments, icons, docs, scores, viewport, colors)
//! stay at their defaults.

use super::codes;
use super::format::{CompiledBoard, CompiledNode, CompiledPin, CompiledVariable, NONE_IDX};
use crate::flow::board::{Board, Layer, LayerCache};
use crate::flow::node::{FnRefs, Node, NodeWasm};
use crate::flow::pin::{Pin, PinOptions};
use crate::flow::variable::Variable;
use crate::state::FlowNodeRegistryInner;
use flow_like_storage::Path;
use flow_like_types::{Result, anyhow, sync::Mutex};
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::time::SystemTime;

/// Reinflate a possibly-interned field: `None` means "identical to the
/// catalog default", which must be present — the artifact is
/// fingerprint-bound to the registry that produced it.
fn inflate(stored: &Option<String>, default_value: Option<&str>, what: &str) -> Result<String> {
    match (stored, default_value) {
        (Some(value), _) => Ok(value.clone()),
        (None, Some(default_value)) => Ok(default_value.to_string()),
        (None, None) => Err(anyhow!(
            "compiled board interned {what} but the catalog default is unavailable"
        )),
    }
}

/// The single catalog-default pin matching (name, direction). Interning only
/// ever happens for unambiguous matches, so ambiguity here means the artifact
/// and registry disagree.
fn catalog_default_pin<'a>(
    default_node: Option<&'a Node>,
    name: &str,
    pin_type: &crate::flow::pin::PinType,
) -> Option<&'a Pin> {
    let default_node = default_node?;
    let mut candidates = default_node
        .pins
        .values()
        .filter(|p| p.name == name && &p.pin_type == pin_type);
    let first = candidates.next()?;
    if candidates.next().is_some() {
        return None;
    }
    Some(first)
}

pub fn reconstruct_board(
    compiled: &CompiledBoard,
    board_dir: Path,
    catalog: Option<&FlowNodeRegistryInner>,
) -> Result<Board> {
    let pin_ids: Vec<&str> = compiled.pins.iter().map(|p| p.id.as_str()).collect();

    let mut layers: HashMap<String, Layer> = HashMap::with_capacity(compiled.layers.len());
    for cl in &compiled.layers {
        let mut layer = Layer::new(
            cl.id.clone(),
            cl.name.clone(),
            codes::layer_type_from(cl.layer_type)?,
        );
        layer.parent_id = (cl.parent_layer != NONE_IDX)
            .then(|| compiled.layers[cl.parent_layer as usize].id.clone());
        layer.cache = cl
            .cache
            .as_ref()
            .map(|c| -> Result<LayerCache> {
                Ok(LayerCache {
                    enabled: c.enabled,
                    prefix: c.prefix.clone(),
                    ttl_seconds: c.ttl_seconds,
                    scope: codes::layer_cache_scope_from(c.scope)?,
                })
            })
            .transpose()?;
        for variable in &cl.variables {
            let v = reconstruct_variable(variable)?;
            layer.variables.insert(v.id.clone(), v);
        }
        for &pin_idx in &cl.pins {
            let cp = compiled.pins.get(pin_idx as usize).ok_or_else(|| {
                anyhow!(
                    "layer {} references pin index {pin_idx} out of range",
                    cl.id
                )
            })?;
            let pin = reconstruct_pin(cp, &pin_ids, None)?;
            layer.pins.insert(pin.id.clone(), pin);
        }
        layers.insert(cl.id.clone(), layer);
    }

    let mut nodes: HashMap<String, Node> = HashMap::new();
    for cn in &compiled.nodes {
        let node = reconstruct_node(cn, compiled, &pin_ids, catalog)?;
        if cn.body_layer == NONE_IDX {
            nodes.insert(node.id.clone(), node);
        } else {
            let layer_id = &compiled.layers[cn.body_layer as usize].id;
            if let Some(layer) = layers.get_mut(layer_id) {
                layer.nodes.insert(node.id.clone(), node);
            }
        }
    }

    let mut variables: HashMap<String, Variable> = HashMap::with_capacity(compiled.variables.len());
    for cv in &compiled.variables {
        let v = reconstruct_variable(cv)?;
        variables.insert(v.id.clone(), v);
    }

    Ok(Board {
        id: compiled.id.clone(),
        name: compiled.name.clone(),
        description: String::new(),
        nodes,
        variables,
        comments: HashMap::new(),
        viewport: (0.0, 0.0, 0.0),
        version: compiled.version,
        stage: codes::stage_from(compiled.stage)?,
        log_level: codes::log_level_from(compiled.log_level)?,
        execution_mode: codes::execution_mode_from(compiled.execution_mode)?,
        refs: compiled.refs.iter().cloned().collect(),
        internal_refs: HashMap::new(),
        layers,
        page_ids: compiled.page_ids.clone(),
        hash: None,
        created_at: SystemTime::UNIX_EPOCH,
        updated_at: SystemTime::UNIX_EPOCH,
        parent: None,
        board_dir,
        logic_nodes: HashMap::new(),
        app_state: None,
        pin_index: None,
    })
}

fn reconstruct_node(
    cn: &CompiledNode,
    compiled: &CompiledBoard,
    pin_ids: &[&str],
    catalog: Option<&FlowNodeRegistryInner>,
) -> Result<Node> {
    let default_node = catalog
        .and_then(|c| c.registry.get(&cn.name))
        .map(|(default_node, _)| default_node);

    let mut pins: HashMap<String, Pin> = HashMap::with_capacity(cn.pins.len());
    for &pin_idx in &cn.pins {
        let cp = compiled
            .pins
            .get(pin_idx as usize)
            .ok_or_else(|| anyhow!("node {} references pin index {pin_idx} out of range", cn.id))?;
        let pin = reconstruct_pin(cp, pin_ids, default_node)?;
        pins.insert(pin.id.clone(), pin);
    }

    Ok(Node {
        id: cn.id.clone(),
        name: cn.name.clone(),
        friendly_name: inflate(
            &cn.friendly_name,
            default_node.map(|d| d.friendly_name.as_str()),
            "node friendly_name",
        )?,
        description: inflate(
            &cn.description,
            default_node.map(|d| d.description.as_str()),
            "node description",
        )?,
        coordinates: None,
        category: inflate(
            &cn.category,
            default_node.map(|d| d.category.as_str()),
            "node category",
        )?,
        scores: None,
        pins,
        start: cn.start.then_some(true),
        icon: None,
        comment: None,
        long_running: cn.long_running.then_some(true),
        error: None,
        docs: None,
        event_callback: cn.event_callback.then_some(true),
        layer: (cn.layer != NONE_IDX).then(|| compiled.layers[cn.layer as usize].id.clone()),
        hash: None,
        fn_refs: cn.fn_refs.as_ref().map(|f| FnRefs {
            fn_refs: f.fn_refs.clone(),
            can_reference_fns: f.can_reference_fns,
            can_be_referenced_by_fns: f.can_be_referenced_by_fns,
        }),
        oauth_providers: (!cn.oauth_providers.is_empty()).then(|| cn.oauth_providers.clone()),
        required_oauth_scopes: (!cn.required_oauth_scopes.is_empty())
            .then(|| cn.required_oauth_scopes.iter().cloned().collect()),
        only_offline: cn.only_offline,
        version: (cn.node_version != u32::MAX).then_some(cn.node_version),
        wasm: cn
            .wasm
            .as_ref()
            .map(|w| -> Result<NodeWasm> {
                Ok(NodeWasm {
                    package_id: w.package_id.clone(),
                    permissions: w
                        .permissions
                        .iter()
                        .map(|p| codes::node_permission_from(*p))
                        .collect::<Result<Vec<_>>>()?,
                })
            })
            .transpose()?,
        namespace: default_node.and_then(|d| d.namespace.clone()),
        alias: default_node.and_then(|d| d.alias.clone()),
        receiver: default_node.and_then(|d| d.receiver.clone()),
    })
}

fn reconstruct_pin(cp: &CompiledPin, pin_ids: &[&str], default_node: Option<&Node>) -> Result<Pin> {
    let map_edges = |edges: &[u32]| -> BTreeSet<String> {
        edges
            .iter()
            .filter_map(|&i| pin_ids.get(i as usize).map(|id| (*id).to_string()))
            .collect()
    };

    let pin_type = codes::pin_type_from(cp.pin_type)?;
    let catalog_pin = catalog_default_pin(default_node, &cp.name, &pin_type);

    Ok(Pin {
        id: cp.id.clone(),
        name: cp.name.clone(),
        friendly_name: inflate(
            &cp.friendly_name,
            catalog_pin.map(|p| p.friendly_name.as_str()),
            "pin friendly_name",
        )?,
        description: inflate(
            &cp.description,
            catalog_pin.map(|p| p.description.as_str()),
            "pin description",
        )?,
        pin_type,
        data_type: codes::variable_type_from(cp.data_type)?,
        schema: cp.schema.clone(),
        value_type: codes::value_type_from(cp.value_type)?,
        depends_on: map_edges(&cp.depends_on),
        connected_to: map_edges(&cp.connected_to),
        default_value: cp.default_value.clone(),
        index: cp.index,
        options: cp.options.as_ref().map(|o| PinOptions {
            sensitive: o.sensitive,
            valid_values: o.valid_values.clone(),
            range: o.range,
            step: o.step,
            enforce_schema: o.enforce_schema,
            enforce_generic_value_type: o.enforce_generic_value_type,
        }),
        value: None,
    })
}

fn reconstruct_variable(cv: &CompiledVariable) -> Result<Variable> {
    Ok(Variable {
        id: cv.id.clone(),
        name: cv.name.clone(),
        category: None,
        description: None,
        default_value: cv.default_value.clone(),
        data_type: codes::variable_type_from(cv.data_type)?,
        value_type: codes::value_type_from(cv.value_type)?,
        exposed: cv.exposed,
        secret: cv.secret,
        editable: cv.editable,
        hash: None,
        schema: cv.schema.clone(),
        runtime_configured: cv.runtime_configured,
        value: Arc::new(Mutex::new(flow_like_types::Value::Null)),
    })
}
