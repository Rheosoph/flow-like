use flow_like::flow::{node::Node, pin::Pin, variable::VariableType};
use flow_like_types::geometry::{GeometryKind, marker};

pub(crate) fn geometry_input<'a>(
    node: &'a mut Node,
    name: &str,
    friendly_name: &str,
    description: &str,
    kind: Option<GeometryKind>,
) -> &'a mut Pin {
    let pin = node.add_input_pin(name, friendly_name, description, VariableType::Geometry);
    pin.schema = kind.map(|kind| marker(kind).to_string());
    pin
}

pub(crate) fn geometry_output<'a>(
    node: &'a mut Node,
    name: &str,
    friendly_name: &str,
    description: &str,
    kind: Option<GeometryKind>,
) -> &'a mut Pin {
    let pin = node.add_output_pin(name, friendly_name, description, VariableType::Geometry);
    pin.schema = kind.map(|kind| marker(kind).to_string());
    pin
}

#[cfg(feature = "execute")]
use {
    super::GeoCoordinate,
    flow_like::flow::execution::context::ExecutionContext,
    flow_like_types::{Result, Value, geometry::validate_geometry, json::json},
};

#[cfg(feature = "execute")]
fn point_coordinate(value: &Value) -> Result<GeoCoordinate> {
    validate_geometry(value, Some(GeometryKind::Point))?;
    let coordinates = &value["coordinates"];
    Ok(GeoCoordinate::new(
        coordinates[1].as_f64().expect("validated latitude"),
        coordinates[0].as_f64().expect("validated longitude"),
    ))
}

#[cfg(feature = "execute")]
pub(crate) async fn coordinate_input(
    context: &ExecutionContext,
    geometry_name: &str,
) -> Result<GeoCoordinate> {
    point_coordinate(&context.evaluate_pin::<Value>(geometry_name).await?)
}

#[cfg(feature = "execute")]
pub(crate) async fn coordinates_input(
    context: &ExecutionContext,
    geometry_name: &str,
) -> Result<Vec<GeoCoordinate>> {
    let points: Vec<Value> = context.evaluate_pin(geometry_name).await?;
    points.iter().map(point_coordinate).collect()
}

#[cfg(feature = "execute")]
pub(crate) fn point_geometry(coordinate: &GeoCoordinate) -> Result<Value> {
    let geometry = json!({
        "type": "Point",
        "coordinates": [coordinate.longitude, coordinate.latitude],
    });
    validate_geometry(&geometry, Some(GeometryKind::Point))?;
    Ok(geometry)
}

#[cfg(feature = "execute")]
pub(crate) fn line_geometry(coordinates: &[GeoCoordinate]) -> Result<Value> {
    let geometry = json!({
        "type": "LineString",
        "coordinates": coordinates.iter()
            .map(|coordinate| [coordinate.longitude, coordinate.latitude])
            .collect::<Vec<_>>(),
    });
    validate_geometry(&geometry, Some(GeometryKind::LineString))?;
    Ok(geometry)
}

#[cfg(feature = "execute")]
pub(crate) async fn clear_output(context: &mut ExecutionContext, name: &str) -> Result<()> {
    let pin = context.get_pin_by_name(name).await?;
    context.clear_pin_override(pin.id());
    pin.reset().await;
    Ok(())
}

#[cfg(all(test, feature = "execute"))]
pub(crate) mod tests {
    use super::*;
    use ahash::AHashMap;
    use flow_like::{
        flow::{
            board::ExecutionStage,
            execution::{LogLevel, internal_node::InternalNode, internal_pin::InternalPin},
            node::NodeLogic,
        },
        profile::Profile,
        state::{FlowLikeConfig, FlowLikeState},
        utils::http::HTTPClient,
    };
    use flow_like_types::sync::{Mutex, RwLock};
    use std::sync::{Arc, Weak};

    pub(crate) async fn execution_context(logic: Arc<dyn NodeLogic>) -> ExecutionContext {
        let node = logic.get_node();
        context_with_node(logic, node).await
    }

    pub(crate) async fn context_with_node(
        logic: Arc<dyn NodeLogic>,
        node: Node,
    ) -> ExecutionContext {
        let mut pins = AHashMap::new();
        let mut names = AHashMap::<String, Vec<Arc<InternalPin>>>::new();
        for pin in node.pins.values() {
            let internal = Arc::new(InternalPin::new(pin, false));
            names
                .entry(pin.name.clone())
                .or_default()
                .push(internal.clone());
            pins.insert(pin.id.clone(), internal);
        }
        let current = Arc::new(InternalNode::new(node, pins, logic, names));
        for pin in current.pins.iter() {
            pin.init_node(Arc::downgrade(&current));
        }
        ExecutionContext::new(
            Arc::new(AHashMap::from_iter([(
                current.node_id().to_owned(),
                current.clone(),
            )])),
            &Weak::new(),
            &Arc::new(FlowLikeState::new(
                FlowLikeConfig::new(),
                HTTPClient::new_without_refetch(),
            )),
            &current,
            &Arc::new(Mutex::new(AHashMap::new())),
            &Arc::new(RwLock::new(AHashMap::new())),
            LogLevel::Debug,
            ExecutionStage::Dev,
            Arc::new(Profile::default()),
            None,
            Arc::new(RwLock::new(Vec::new())),
            None,
            None,
            Arc::new(AHashMap::new()),
            None,
        )
        .await
    }

    #[tokio::test]
    async fn location_point_connects_directly_and_rejects_unset_or_invalid_geometry() {
        use crate::geo::{
            location::GetCurrentLocationNode, search::reverse_geocode::ReverseGeocodeNode,
        };

        let mut location = execution_context(Arc::new(GetCurrentLocationNode)).await;
        let mut reverse = execution_context(Arc::new(ReverseGeocodeNode::new())).await;
        assert!(reverse.get_pin_by_name("coordinate").await.is_err());
        assert!(location.get_pin_by_name("coordinate").await.is_err());

        let output = location.get_pin_by_name("geometry").await.unwrap();
        let input = reverse.get_pin_by_name("geometry").await.unwrap();
        input.init_depends_on(vec![Arc::downgrade(&output)]);
        // The connected producer has not supplied a location yet.
        assert!(coordinate_input(&reverse, "geometry").await.is_err());
        location
            .set_pin_value(
                "geometry",
                json!({"type":"Point","coordinates":[13.405,52.52]}),
            )
            .await
            .unwrap();
        let point = coordinate_input(&reverse, "geometry").await.unwrap();
        assert_eq!((point.longitude, point.latitude), (13.405, 52.52));

        for invalid in [
            json!(null),
            json!({"type":"LineString","coordinates":[[1,2],[3,4]]}),
            json!({"type":"Point","coordinates":[200,52]}),
        ] {
            reverse.override_pin_value(input.id(), invalid);
            assert!(coordinate_input(&reverse, "geometry").await.is_err());
        }
        reverse.override_pin_value(input.id(), json!({"type":"Point","coordinates":[-74,40.7]}));
        let point = coordinate_input(&reverse, "geometry").await.unwrap();
        assert_eq!((point.longitude, point.latitude), (-74.0, 40.7));
    }

    #[tokio::test]
    async fn clearing_geometry_removes_shared_and_function_scoped_values() {
        use crate::geo::location::GetCurrentLocationNode;
        let mut context = execution_context(Arc::new(GetCurrentLocationNode)).await;
        let output = context.get_pin_by_name("geometry").await.unwrap();
        let point = json!({"type":"Point","coordinates":[0,0]});
        context.override_pin_value(output.id(), point.clone());
        context.set_pin_value("geometry", point).await.unwrap();
        clear_output(&mut context, "geometry").await.unwrap();
        assert!(output.get_raw_value().await.is_none());
        assert!(context.evaluate_pin::<Value>("geometry").await.is_err());
    }
}
