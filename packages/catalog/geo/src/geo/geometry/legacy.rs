use crate::geo::{
    GeoCoordinate, h3::cells_to_multi_polygon::Polygon, routing::osrm::RouteGeometry,
};
use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, dynamic_pin_source_literal},
    pin::{Pin, PinOptions, ValueType},
    variable::VariableType,
};
#[cfg(feature = "execute")]
use flow_like_types::Value;
use flow_like_types::{
    Result, async_trait,
    geometry::{GeometryKind, marker},
    json::json,
};

const MODES: &[&str] = &[
    "coordinate_to_point",
    "coordinates_to_points",
    "waypoints_to_points",
    "point_to_coordinate",
    "geometry_to_route",
    "geometry_to_trip_route",
    "h3_boundary",
    "h3_polygons",
];

fn set_type(pin: &mut Pin, data_type: VariableType, value_type: ValueType) {
    if pin.data_type != data_type || pin.value_type != value_type {
        pin.default_value = None;
    }
    pin.data_type = data_type;
    pin.value_type = value_type;
    pin.schema = None;
}

fn specialize(node: &mut Node, mode: &str) -> Result<()> {
    if !MODES.contains(&mode) {
        return Err(flow_like_types::anyhow!(
            "Unknown legacy Geometry conversion: {mode}"
        ));
    }
    for pin in node.pins.values_mut() {
        match pin.name.as_str() {
            "value" => match mode {
                "coordinate_to_point" | "coordinates_to_points" | "waypoints_to_points" => {
                    let container = if mode == "coordinates_to_points" {
                        ValueType::Array
                    } else {
                        ValueType::Normal
                    };
                    set_type(pin, VariableType::Struct, container);
                    pin.set_schema::<GeoCoordinate>();
                }
                _ => {
                    set_type(pin, VariableType::Geometry, ValueType::Normal);
                    pin.schema = match mode {
                        "point_to_coordinate" => Some(marker(GeometryKind::Point).to_owned()),
                        "geometry_to_route" | "geometry_to_trip_route" => {
                            Some(marker(GeometryKind::LineString).to_owned())
                        }
                        "h3_polygons" => Some(marker(GeometryKind::MultiPolygon).to_owned()),
                        _ => None,
                    };
                }
            },
            "converted" => match mode {
                "coordinate_to_point" | "coordinates_to_points" | "waypoints_to_points" => {
                    let container = if mode == "coordinate_to_point" {
                        ValueType::Normal
                    } else {
                        ValueType::Array
                    };
                    set_type(pin, VariableType::Geometry, container);
                    pin.schema = Some(marker(GeometryKind::Point).to_owned());
                }
                _ => {
                    let container = if mode == "geometry_to_trip_route" {
                        ValueType::Array
                    } else {
                        ValueType::Normal
                    };
                    set_type(pin, VariableType::Struct, container);
                    match mode {
                        "geometry_to_route" => {
                            pin.set_schema::<RouteGeometry>();
                        }
                        "h3_polygons" => {
                            pin.set_schema::<Polygon>();
                        }
                        _ => {
                            pin.set_schema::<GeoCoordinate>();
                        }
                    }
                }
            },
            "source" => {
                let source_type = if matches!(mode, "h3_boundary" | "h3_polygons") {
                    VariableType::String
                } else {
                    VariableType::Generic
                };
                let container = if mode == "h3_polygons" {
                    ValueType::Array
                } else {
                    ValueType::Normal
                };
                set_type(pin, source_type, container);
                pin.set_options(
                    PinOptions::new()
                        .set_optional(!matches!(mode, "h3_boundary" | "h3_polygons"))
                        .build(),
                );
            }
            _ => {}
        }
    }
    Ok(())
}

#[crate::register_node]
#[derive(Default)]
pub struct GeometryLegacyAdapterNode;

#[async_trait]
impl NodeLogic for GeometryLegacyAdapterNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "geometry_legacy_adapter",
            "Legacy Geo Adapter",
            "Preserves connections to historical coordinate and route data when a saved board upgrades to Geometry pins.",
            "Web/Geo/Geometry",
        );
        node.set_version(1);
        node.set_flowscript_name("geometry", "legacyAdapter");
        node.add_icon("/flow/icons/map.svg");
        node.add_input_pin(
            "mode",
            "Conversion",
            "Historical payload conversion",
            VariableType::String,
        )
        .set_default_value(Some(json!("coordinate_to_point")))
        .set_options(
            PinOptions::new()
                .set_valid_values(MODES.iter().map(|mode| (*mode).to_owned()).collect())
                .build(),
        );
        node.add_input_pin("value", "Value", "Value to convert", VariableType::Struct);
        node.add_input_pin(
            "source",
            "Original Source",
            "Original H3 cell or cells used to preserve historical boundaries",
            VariableType::Generic,
        )
        .set_options(PinOptions::new().set_optional(true).build());
        node.add_output_pin(
            "converted",
            "Converted",
            "Converted historical value or Geometry",
            VariableType::Geometry,
        );
        specialize(&mut node, "coordinate_to_point").expect("known conversion mode");
        node.set_long_running(false);
        node
    }

    async fn on_update(&self, node: &mut Node, _board: &Board) {
        let Some(mode) = dynamic_pin_source_literal(node, "mode") else {
            return;
        };
        if let Err(error) = specialize(node, &mode) {
            node.error = Some(error.to_string());
        }
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        crate::geo::pins::clear_output(context, "converted").await?;
        let mode: String = context.evaluate_pin("mode").await?;
        let value: Value = context.evaluate_pin("value").await?;
        // Geometry is evaluated first so the producer completes before its original H3 input is read.
        let source = if matches!(mode.as_str(), "h3_boundary" | "h3_polygons") {
            Some(context.evaluate_pin::<Value>("source").await?)
        } else {
            None
        };
        let converted = convert(&mode, &value, source.as_ref())?;
        context.set_pin_value("converted", converted).await
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> Result<()> {
        Err(flow_like_types::anyhow!(
            "Legacy Geometry conversions require the execute feature"
        ))
    }
}

#[cfg(feature = "execute")]
fn coordinate(value: &Value) -> Result<GeoCoordinate> {
    flow_like_types::geometry::validate_geometry(value, Some(GeometryKind::Point))?;
    Ok(GeoCoordinate::new(
        value["coordinates"][1]
            .as_f64()
            .expect("validated latitude"),
        value["coordinates"][0]
            .as_f64()
            .expect("validated longitude"),
    ))
}

#[cfg(feature = "execute")]
fn convert(mode: &str, value: &Value, source: Option<&Value>) -> Result<Value> {
    use flow_like_types::{geometry::validate_geometry, json::from_value};
    use std::str::FromStr;
    match mode {
        "coordinate_to_point" => {
            crate::geo::pins::point_geometry(&from_value::<GeoCoordinate>(value.clone())?)
        }
        "coordinates_to_points" | "waypoints_to_points" => {
            let coordinates: Vec<GeoCoordinate> = from_value(value.clone())?;
            let points = coordinates
                .iter()
                .map(crate::geo::pins::point_geometry)
                .collect::<Result<Vec<_>>>()?;
            Ok(json!(points))
        }
        "point_to_coordinate" => Ok(json!(coordinate(value)?)),
        "geometry_to_route" | "geometry_to_trip_route" => {
            validate_geometry(value, Some(GeometryKind::LineString))?;
            let points = value["coordinates"]
                .as_array()
                .expect("validated positions")
                .iter()
                .map(|position| {
                    GeoCoordinate::new(
                        position[1].as_f64().expect("validated latitude"),
                        position[0].as_f64().expect("validated longitude"),
                    )
                })
                .collect();
            Ok(json!(RouteGeometry { points }))
        }
        "h3_boundary" => {
            validate_geometry(value, None)?;
            let source = source
                .and_then(Value::as_str)
                .ok_or_else(|| flow_like_types::anyhow!("Original H3 cell is required"))?;
            let cell = h3o::CellIndex::from_str(source)?;
            let coordinates = cell
                .boundary()
                .iter()
                .map(|position| GeoCoordinate::new(position.lat(), position.lng()))
                .collect::<Vec<_>>();
            Ok(json!(coordinates))
        }
        "h3_polygons" => {
            use h3o::{CellIndex, geom::SolventBuilder};
            validate_geometry(value, Some(GeometryKind::MultiPolygon))?;
            let sources: Vec<String> = from_value(
                source
                    .cloned()
                    .ok_or_else(|| flow_like_types::anyhow!("Original H3 cells are required"))?,
            )?;
            let cells = sources
                .iter()
                .filter_map(|source| CellIndex::from_str(source).ok())
                .collect::<Vec<_>>();
            if cells.is_empty() {
                return Ok(json!([]));
            }
            let polygons = SolventBuilder::new()
                .build()
                .dissolve(cells)
                .map_err(|error| {
                    flow_like_types::anyhow!("Failed to restore H3 polygons: {error}")
                })?;
            let polygons = polygons
                .0
                .iter()
                .map(|polygon| Polygon {
                    exterior: polygon
                        .exterior()
                        .coords()
                        .map(|position| GeoCoordinate::new(position.y, position.x))
                        .collect(),
                    interiors: polygon
                        .interiors()
                        .iter()
                        .map(|ring| {
                            ring.coords()
                                .map(|position| GeoCoordinate::new(position.y, position.x))
                                .collect()
                        })
                        .collect(),
                })
                .collect::<Vec<_>>();
            Ok(json!(polygons))
        }
        _ => Err(flow_like_types::anyhow!(
            "Unknown legacy Geometry conversion: {mode}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_conversion_declares_its_native_and_historical_pin_types() {
        for mode in MODES {
            let mut node = GeometryLegacyAdapterNode.get_node();
            specialize(&mut node, mode).unwrap();
            let input = node.get_pin_by_name("value").unwrap();
            let output = node.get_pin_by_name("converted").unwrap();
            assert_eq!(
                node.get_pin_by_name("source").unwrap().is_optional(),
                !matches!(*mode, "h3_boundary" | "h3_polygons")
            );
            if mode.ends_with("to_point") || mode.ends_with("to_points") {
                assert_eq!(input.data_type, VariableType::Struct);
                assert_eq!(output.data_type, VariableType::Geometry);
                assert_eq!(output.schema.as_deref(), Some(marker(GeometryKind::Point)));
            } else {
                assert_eq!(input.data_type, VariableType::Geometry);
                assert_eq!(output.data_type, VariableType::Struct);
            }
            if *mode == "waypoints_to_points" {
                assert_eq!(input.value_type, ValueType::Normal);
                assert_eq!(output.value_type, ValueType::Array);
            }
            if *mode == "geometry_to_trip_route" {
                assert_eq!(output.value_type, ValueType::Array);
                assert_eq!(
                    flow_like_types::json::from_str::<flow_like_types::Value>(
                        output.schema.as_ref().unwrap()
                    )
                    .unwrap(),
                    flow_like_types::json::to_value(schemars::schema_for!(GeoCoordinate)).unwrap()
                );
            }
        }
    }

    #[cfg(feature = "execute")]
    #[test]
    fn coordinate_and_route_conversions_reproduce_historical_values() {
        let point = json!({"type":"Point","coordinates":[13.405,52.52]});
        let old = json!({"latitude":52.52,"longitude":13.405});
        assert_eq!(convert("coordinate_to_point", &old, None).unwrap(), point);
        assert_eq!(convert("point_to_coordinate", &point, None).unwrap(), old);
        for mode in ["coordinates_to_points", "waypoints_to_points"] {
            assert_eq!(convert(mode, &json!([old]), None).unwrap(), json!([point]));
            assert_eq!(convert(mode, &json!([]), None).unwrap(), json!([]));
        }
        let line = json!({"type":"LineString","coordinates":[[13.405,52.52],[13.4,52.5]]});
        for mode in ["geometry_to_route", "geometry_to_trip_route"] {
            assert_eq!(
                convert(mode, &line, None).unwrap(),
                json!({"points":[old,{"latitude":52.5,"longitude":13.4}]})
            );
        }
    }

    #[cfg(feature = "execute")]
    #[test]
    fn legacy_conversions_reject_invalid_geometry_and_coordinates() {
        for mode in MODES {
            assert!(convert(mode, &Value::Null, None).is_err(), "{mode}");
        }
        assert!(
            convert(
                "coordinate_to_point",
                &json!({"latitude":91.0,"longitude":0.0}),
                None
            )
            .is_err()
        );
        assert!(
            convert(
                "point_to_coordinate",
                &json!({"type":"LineString","coordinates":[[0,0],[1,1]]}),
                None
            )
            .is_err()
        );
        assert!(convert("unknown", &json!({}), None).is_err());
    }

    #[cfg(feature = "execute")]
    #[test]
    fn h3_conversions_restore_raw_antimeridian_payloads() {
        use h3o::{CellIndex, geom::SolventBuilder};
        use std::str::FromStr;
        let cell = CellIndex::from_str("840d9edffffffff").unwrap();
        let boundary = super::super::integrations::h3_boundary_geometry(cell).unwrap();
        let restored = convert("h3_boundary", &boundary, Some(&json!(cell.to_string()))).unwrap();
        let expected = cell
            .boundary()
            .iter()
            .map(|position| GeoCoordinate::new(position.lat(), position.lng()))
            .collect::<Vec<_>>();
        assert_eq!(restored, json!(expected));
        assert_ne!(
            restored.as_array().unwrap().first(),
            restored.as_array().unwrap().last()
        );
        let original = SolventBuilder::new().build().dissolve(vec![cell]).unwrap();
        let geometry =
            super::super::integrations::h3_multipolygon_geometry(original.clone()).unwrap();
        let restored = convert(
            "h3_polygons",
            &geometry,
            Some(&json!([cell.to_string(), "invalid"])),
        )
        .unwrap();
        let expected_exterior = original.0[0]
            .exterior()
            .coords()
            .map(|position| GeoCoordinate::new(position.y, position.x))
            .collect::<Vec<_>>();
        let expected_exterior = json!(expected_exterior);
        let expected_ring = expected_exterior.as_array().unwrap();
        let restored_ring = restored[0]["exterior"].as_array().unwrap();
        assert_eq!(restored_ring.first(), restored_ring.last());
        assert_eq!(restored_ring.len(), expected_ring.len());
        // H3 dissolve chooses a ring start through hash iteration; preserve its raw edges.
        let ring_len = expected_ring.len() - 1;
        assert!((0..ring_len).any(|offset| {
            (0..ring_len)
                .all(|index| restored_ring[index] == expected_ring[(index + offset) % ring_len])
        }));
        assert!(restored_ring.windows(2).any(|edge| {
            (edge[0]["longitude"].as_f64().unwrap() - edge[1]["longitude"].as_f64().unwrap()).abs()
                > 180.0
        }));
        assert_eq!(restored.as_array().unwrap().len(), original.0.len());
        assert_eq!(
            convert(
                "h3_polygons",
                &json!({"type":"MultiPolygon","coordinates":[]}),
                Some(&json!(["invalid"]))
            )
            .unwrap(),
            json!([])
        );
    }

    #[cfg(feature = "execute")]
    #[tokio::test]
    async fn runtime_conversion_requires_producer_geometry_and_clears_stale_output() {
        use crate::geo::pins::tests::context_with_node;
        use std::sync::Arc;
        let logic: Arc<dyn NodeLogic> = Arc::new(GeometryLegacyAdapterNode);
        let mut node = logic.get_node();
        node.pins
            .values_mut()
            .find(|pin| pin.name == "mode")
            .unwrap()
            .set_default_value(Some(json!("h3_boundary")));
        specialize(&mut node, "h3_boundary").unwrap();
        let mut context = context_with_node(logic.clone(), node).await;
        context
            .set_pin_value("source", json!("840d9edffffffff"))
            .await
            .unwrap();
        assert!(logic.run(&mut context).await.is_err());
        context
            .set_pin_value(
                "value",
                json!({"type":"Polygon","coordinates":[[[0,0],[1,0],[0,1],[0,0]]]}),
            )
            .await
            .unwrap();
        logic.run(&mut context).await.unwrap();
        assert!(
            context
                .evaluate_pin::<Value>("converted")
                .await
                .unwrap()
                .is_array()
        );
        context
            .get_pin_by_name("value")
            .await
            .unwrap()
            .reset()
            .await;
        assert!(logic.run(&mut context).await.is_err());
        assert!(
            context
                .get_pin_by_name("converted")
                .await
                .unwrap()
                .get_raw_value()
                .await
                .is_none()
        );
    }
}
