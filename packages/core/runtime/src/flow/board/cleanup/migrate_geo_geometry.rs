use std::collections::{HashMap, HashSet};

use crate::{
    flow::{
        board::Board,
        node::Node,
        pin::{Pin, PinType, geometry_pins_are_compatible},
        variable::{VariableType, validate_typed_default},
    },
    state::FlowNodeRegistryInner,
};
use flow_like_types::{Result, Value, anyhow, geometry::GeometryKind, json::json};

struct Mapping {
    old: &'static str,
    native: &'static str,
    mode: &'static str,
    source: Option<&'static str>,
}

fn mappings(node: &str) -> &'static [Mapping] {
    macro_rules! mapping {
        ($old:literal, $native:literal, $mode:literal) => {
            Mapping {
                old: $old,
                native: $native,
                mode: $mode,
                source: None,
            }
        };
        ($old:literal, $native:literal, $mode:literal, $source:literal) => {
            Mapping {
                old: $old,
                native: $native,
                mode: $mode,
                source: Some($source),
            }
        };
    }
    match node {
        "geo_reverse_geocode" | "geo_get_map_image" | "h3_latlng_to_cell" | "geo_osrm_nearest" => {
            &[mapping!("coordinate", "geometry", "coordinate_to_point")]
        }
        "h3_cell_to_latlng" => &[mapping!(
            "coordinate",
            "geometry_out",
            "point_to_coordinate"
        )],
        "geo_get_current_location" => &[mapping!("coordinate", "geometry", "point_to_coordinate")],
        "h3_cell_to_boundary" => &[mapping!("boundary", "geometry_out", "h3_boundary", "cell")],
        "h3_cells_to_multi_polygon" => {
            &[mapping!("polygons", "geometry_out", "h3_polygons", "cells")]
        }
        "geo_plan_route" => &[
            mapping!("start", "start_geometry", "coordinate_to_point"),
            mapping!("end", "end_geometry", "coordinate_to_point"),
            mapping!("waypoints", "waypoint_geometries", "waypoints_to_points"),
            mapping!("geometry", "geometry_out", "geometry_to_route"),
        ],
        "geo_osrm_table" | "geo_osrm_match_trace" => &[mapping!(
            "coordinates",
            "geometries",
            "coordinates_to_points"
        )],
        "geo_osrm_trip" => &[
            mapping!("coordinates", "geometries", "coordinates_to_points"),
            mapping!("geometry", "geometry_out", "geometry_to_trip_route"),
        ],
        _ => &[],
    }
}

#[derive(Clone)]
struct Location {
    node: String,
    layer: Option<String>,
}

struct Change {
    location: Location,
    old: Pin,
    native: Pin,
    mode: &'static str,
    source: Option<Pin>,
    native_supplied: bool,
    converted_default: Result<Option<Vec<u8>>>,
}

fn input_supplied(pin: &Pin) -> bool {
    !pin.depends_on.is_empty()
        || pin.default_value.as_ref().is_some_and(|bytes| {
            !bytes.is_empty()
                && !flow_like_types::json::from_slice::<Value>(bytes)
                    .is_ok_and(|value| value.is_null())
        })
}

fn convert_default(pin: &Pin, native: &Pin, mode: &str) -> Result<Option<Vec<u8>>> {
    let Some(bytes) = &pin.default_value else {
        return Ok(None);
    };
    if bytes.is_empty() {
        return Ok(None);
    }
    if bytes.len() > flow_like_types::geometry::MAX_GEOMETRY_BYTES {
        return Err(anyhow!(
            "Legacy coordinate default exceeds the geometry byte limit"
        ));
    }
    let value: Value = flow_like_types::json::from_slice(bytes)?;
    let point = |coordinate: &Value| -> Result<Value> {
        let latitude = coordinate
            .get("latitude")
            .and_then(Value::as_f64)
            .ok_or_else(|| anyhow!("Legacy coordinate is missing latitude"))?;
        let longitude = coordinate
            .get("longitude")
            .and_then(Value::as_f64)
            .ok_or_else(|| anyhow!("Legacy coordinate is missing longitude"))?;
        let value = json!({"type": "Point", "coordinates": [longitude, latitude]});
        flow_like_types::geometry::validate_geometry(&value, Some(GeometryKind::Point))?;
        Ok(value)
    };
    let value = if value.is_null() {
        Value::Null
    } else if mode == "coordinate_to_point" {
        point(&value)?
    } else {
        Value::Array(
            value
                .as_array()
                .ok_or_else(|| anyhow!("Legacy coordinates must be an array"))?
                .iter()
                .map(point)
                .collect::<Result<Vec<_>>>()?,
        )
    };
    let bytes = flow_like_types::json::to_vec(&value)?;
    validate_typed_default(
        &native.data_type,
        &native.value_type,
        native.schema.as_deref(),
        Some(&bytes),
    )?;
    Ok(Some(bytes))
}

fn collect_changes(board: &Board, registry: &FlowNodeRegistryInner) -> Vec<Change> {
    let mut nodes: Vec<_> = board
        .nodes
        .values()
        .map(|node| (None, node))
        .chain(board.layers.values().flat_map(|layer| {
            layer
                .nodes
                .values()
                .map(|node| (Some(layer.id.clone()), node))
        }))
        .filter(|(_, node)| {
            mappings(&node.name).iter().any(|mapping| {
                node.get_pin_by_name(mapping.old)
                    .is_some_and(|pin| pin.data_type == VariableType::Struct)
            })
        })
        .collect();
    if nodes.is_empty() {
        return Vec::new();
    }
    nodes.sort_by(|(_, left), (_, right)| left.id.cmp(&right.id));
    let connected_targets: HashSet<_> = board
        .nodes
        .values()
        .flat_map(|node| node.pins.values())
        .chain(board.layers.values().flat_map(|layer| {
            layer
                .pins
                .values()
                .chain(layer.nodes.values().flat_map(|node| node.pins.values()))
        }))
        .flat_map(|pin| pin.connected_to.iter().cloned())
        .collect();
    let mut changes = Vec::new();
    for (layer, node) in nodes {
        let Ok(catalog) = registry.get_node(&node.name) else {
            continue;
        };
        if catalog.version.unwrap_or(0) <= node.version.unwrap_or(0) {
            continue;
        }
        for mapping in mappings(&node.name) {
            let Some(old) = node
                .get_pin_by_name(mapping.old)
                .filter(|pin| pin.data_type == VariableType::Struct)
            else {
                continue;
            };
            let Some(canonical) = catalog
                .get_pin_by_name(mapping.native)
                .filter(|pin| pin.data_type == VariableType::Geometry)
            else {
                continue;
            };
            // An older catalog still exposing the Struct pin has not opted into this migration.
            if catalog.get_pin_by_name(mapping.old).is_some() {
                continue;
            }
            let existing = node.get_pin_by_name(mapping.native);
            let mut native = existing.cloned().unwrap_or_else(|| {
                let mut pin = canonical.clone();
                pin.id = format!("{}-geometry", old.id);
                pin
            });
            native.data_type = canonical.data_type.clone();
            native.value_type = canonical.value_type.clone();
            native.schema = canonical.schema.clone();
            native.options = canonical.options.clone();
            native.index = canonical.index;
            let native_supplied = existing
                .is_some_and(|pin| input_supplied(pin) || connected_targets.contains(&pin.id));
            let converted_default = if old.pin_type == PinType::Input {
                convert_default(old, &native, mapping.mode)
            } else {
                Ok(None)
            };
            changes.push(Change {
                location: Location {
                    node: node.id.clone(),
                    layer: layer.clone(),
                },
                old: old.clone(),
                native,
                mode: mapping.mode,
                source: mapping
                    .source
                    .and_then(|name| node.get_pin_by_name(name).cloned()),
                native_supplied,
                converted_default,
            });
        }
    }
    changes
}

fn node_mut<'a>(board: &'a mut Board, location: &Location) -> &'a mut Node {
    match &location.layer {
        Some(layer) => board
            .layers
            .get_mut(layer)
            .unwrap()
            .nodes
            .get_mut(&location.node)
            .unwrap(),
        None => board.nodes.get_mut(&location.node).unwrap(),
    }
}

fn pin_mut<'a>(board: &'a mut Board, id: &str) -> Option<&'a mut Pin> {
    for node in board.nodes.values_mut() {
        if let Some(pin) = node.pins.get_mut(id) {
            return Some(pin);
        }
    }
    for layer in board.layers.values_mut() {
        if let Some(pin) = layer.pins.get_mut(id) {
            return Some(pin);
        }
        for node in layer.nodes.values_mut() {
            if let Some(pin) = node.pins.get_mut(id) {
                return Some(pin);
            }
        }
    }
    None
}

fn connect(board: &mut Board, source: &str, target: &str) {
    if let Some(pin) = pin_mut(board, source) {
        pin.connected_to.insert(target.to_string());
    }
    if let Some(pin) = pin_mut(board, target) {
        pin.depends_on.insert(source.to_string());
    }
}

fn disconnect(board: &mut Board, source: &str, target: &str) {
    if let Some(pin) = pin_mut(board, source) {
        pin.connected_to.remove(target);
    }
    if let Some(pin) = pin_mut(board, target) {
        pin.depends_on.remove(source);
    }
}

fn attach_adapter(
    board: &mut Board,
    registry: &FlowNodeRegistryInner,
    change: &Change,
    old: Pin,
) -> Result<()> {
    let mut adapter = registry.get_node("geometry_legacy_adapter")?;
    adapter.id = format!("geo-migration-{}", old.id);
    if board.nodes.contains_key(&adapter.id)
        || board
            .layers
            .values()
            .any(|layer| layer.nodes.contains_key(&adapter.id))
    {
        return Err(anyhow!(
            "Geometry migration adapter ID already exists: {}",
            adapter.id
        ));
    }
    let owner = node_mut(board, &change.location);
    adapter.layer = change
        .location
        .layer
        .clone()
        .or_else(|| owner.layer.clone());
    adapter.coordinates = owner.coordinates.map(|(x, y, z)| {
        (
            x + if old.pin_type == PinType::Input {
                -260.0
            } else {
                260.0
            },
            y + f32::from(old.index) * 90.0,
            z,
        )
    });
    for pin in adapter.pins.values_mut() {
        pin.id = format!("{}-{}", adapter.id, pin.name);
    }
    adapter.pins = adapter
        .pins
        .into_values()
        .map(|pin| (pin.id.clone(), pin))
        .collect();
    adapter
        .get_pin_mut_by_name("mode")
        .ok_or_else(|| anyhow!("Geometry adapter mode pin is missing"))?
        .set_default_value(Some(json!(change.mode)));

    let input = old.pin_type == PinType::Input;
    let value_index = adapter
        .get_pin_by_name("value")
        .ok_or_else(|| anyhow!("Geometry adapter value pin is missing"))?
        .index;
    let converted_index = adapter
        .get_pin_by_name("converted")
        .ok_or_else(|| anyhow!("Geometry adapter converted pin is missing"))?
        .index;
    let mut value = if input {
        old.clone()
    } else {
        change.native.clone()
    };
    value.name = "value".to_string();
    value.index = value_index;
    value.pin_type = PinType::Input;
    value.connected_to.clear();
    if !input {
        value.id = format!("{}-value", adapter.id);
        value.depends_on.clear();
        value.default_value = None;
    }
    let mut converted = if input { change.native.clone() } else { old };
    converted.name = "converted".to_string();
    converted.index = converted_index;
    converted.pin_type = PinType::Output;
    converted.depends_on.clear();
    if input {
        converted.id = format!("{}-converted", adapter.id);
        converted.connected_to.clear();
        converted.default_value = None;
    }
    let value_id = value.id.clone();
    let converted_id = converted.id.clone();
    adapter
        .pins
        .retain(|_, pin| pin.name != "value" && pin.name != "converted");
    adapter.pins.insert(value.id.clone(), value);
    adapter.pins.insert(converted.id.clone(), converted);
    let mut source_links = Vec::new();
    if let Some(source) = &change.source {
        let mut source = board
            .get_pin_by_id(&source.id)
            .ok_or_else(|| anyhow!("Original H3 source pin is missing: {}", source.id))?
            .clone();
        source.id = format!("{}-source", adapter.id);
        source.name = "source".to_string();
        source.options.get_or_insert_default().optional = Some(false);
        source.index = adapter
            .get_pin_by_name("source")
            .ok_or_else(|| anyhow!("Geometry adapter source pin is missing"))?
            .index;
        source.connected_to.clear();
        source_links.extend(
            source
                .depends_on
                .iter()
                .map(|id| (id.clone(), source.id.clone())),
        );
        adapter.pins.retain(|_, pin| pin.name != "source");
        adapter.pins.insert(source.id.clone(), source);
    }
    for pin in adapter.pins.values() {
        if board.get_pin_by_id(&pin.id).is_some() {
            return Err(anyhow!(
                "Geometry migration pin ID already exists: {}",
                pin.id
            ));
        }
    }
    match &change.location.layer {
        Some(layer) => {
            board
                .layers
                .get_mut(layer)
                .unwrap()
                .nodes
                .insert(adapter.id.clone(), adapter);
        }
        None => {
            board.nodes.insert(adapter.id.clone(), adapter);
        }
    }
    for (source, target) in source_links {
        connect(board, &source, &target);
    }
    if input {
        connect(board, &converted_id, &change.native.id);
    } else {
        connect(board, &change.native.id, &value_id);
    }
    Ok(())
}

fn apply_changes(
    board: &mut Board,
    registry: &FlowNodeRegistryInner,
    changes: &[Change],
) -> Result<()> {
    // Normalize reciprocal references before moving pins between owners.
    let all_pins: Vec<_> = board
        .nodes
        .values()
        .flat_map(|node| node.pins.values())
        .chain(board.layers.values().flat_map(|layer| {
            layer
                .pins
                .values()
                .chain(layer.nodes.values().flat_map(|node| node.pins.values()))
        }))
        .cloned()
        .collect();
    for pin in &all_pins {
        for source in &pin.depends_on {
            connect(board, source, &pin.id);
        }
        for target in &pin.connected_to {
            connect(board, &pin.id, target);
        }
    }
    for change in changes {
        let mut native = change.native.clone();
        if board.get_pin_by_id(&native.id).is_some()
            && !node_mut(board, &change.location)
                .get_pin_by_name(&native.name)
                .is_some_and(|pin| pin.id == native.id)
        {
            return Err(anyhow!(
                "Geometry migration pin ID already exists: {}",
                native.id
            ));
        }
        if native.pin_type == PinType::Input && !change.native_supplied {
            if let Ok(default) = &change.converted_default {
                native.default_value = default.clone();
            }
        }
        if let Some(live) = board.get_pin_by_id(&native.id) {
            native.depends_on = live.depends_on.clone();
            native.connected_to = live.connected_to.clone();
        }
        node_mut(board, &change.location)
            .pins
            .insert(native.id.clone(), native);
    }
    let by_old: HashMap<_, _> = changes
        .iter()
        .map(|change| (change.old.id.as_str(), change))
        .collect();
    for change in changes
        .iter()
        .filter(|change| change.old.pin_type == PinType::Input)
    {
        let sources = board
            .get_pin_by_id(&change.old.id)
            .unwrap()
            .depends_on
            .clone();
        for source in sources {
            if change.native_supplied {
                disconnect(board, &source, &change.old.id);
            } else if let Some(output) = by_old.get(source.as_str())
                && output.old.pin_type == PinType::Output
                && change.converted_default.is_ok()
                && geometry_pins_are_compatible(&output.native, &change.native, &board.refs)?
            {
                disconnect(board, &source, &change.old.id);
                connect(board, &output.native.id, &change.native.id);
            }
        }
    }
    for change in changes {
        let old = node_mut(board, &change.location)
            .pins
            .remove(&change.old.id)
            .unwrap();
        let needs_adapter = match old.pin_type {
            PinType::Input => {
                !change.native_supplied
                    && (!old.depends_on.is_empty() || change.converted_default.is_err())
            }
            PinType::Output => !old.connected_to.is_empty(),
        };
        if needs_adapter {
            attach_adapter(board, registry, change, old)?;
        }
    }
    board.validate_geometry_contracts()?;
    Ok(())
}

/// Move retired Geo Struct pins off their nodes before generic schema synchronization.
/// Failed migrations retain the whole original graph and block only the affected node upgrades.
pub(super) fn migrate_geo_geometry(
    board: &mut Board,
    registry: &FlowNodeRegistryInner,
) -> HashSet<String> {
    let changes = collect_changes(board, registry);
    if changes.is_empty() {
        return HashSet::new();
    }
    let mut migrated = board.clone();
    match apply_changes(&mut migrated, registry, &changes) {
        Ok(()) => {
            *board = migrated;
            HashSet::new()
        }
        Err(error) => {
            let mut blocked = HashSet::new();
            for change in changes {
                let node = node_mut(board, &change.location);
                node.error = Some(format!("Geometry pin migration failed: {error}"));
                blocked.insert(node.id.clone());
            }
            blocked
        }
    }
}
