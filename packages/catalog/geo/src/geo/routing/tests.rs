#[cfg(all(test, feature = "execute"))]
mod routing_tests {
    use crate::geo::{
        GeoCoordinate,
        routing::osrm::{RouteLeg, RouteProfile, RouteResult, RouteStep},
    };

    #[test]
    fn test_route_profile_default() {
        let profile: RouteProfile = RouteProfile::default();
        assert!(matches!(profile, RouteProfile::Car));
    }

    #[test]
    fn test_route_profile_to_string() {
        let car_str = match RouteProfile::Car {
            RouteProfile::Car => "driving",
            RouteProfile::Bike => "cycling",
            RouteProfile::Foot => "foot",
        };
        assert_eq!(car_str, "driving");

        let bike_str = match RouteProfile::Bike {
            RouteProfile::Car => "driving",
            RouteProfile::Bike => "cycling",
            RouteProfile::Foot => "foot",
        };
        assert_eq!(bike_str, "cycling");

        let foot_str = match RouteProfile::Foot {
            RouteProfile::Car => "driving",
            RouteProfile::Bike => "cycling",
            RouteProfile::Foot => "foot",
        };
        assert_eq!(foot_str, "foot");
    }

    #[test]
    fn test_build_osrm_url() {
        let start = GeoCoordinate::new(52.52, 13.405);
        let end = GeoCoordinate::new(52.53, 13.41);
        let waypoints: Vec<GeoCoordinate> = vec![];
        let profile = RouteProfile::Car;
        let alternatives = false;

        let profile_str = match profile {
            RouteProfile::Car => "driving",
            RouteProfile::Bike => "cycling",
            RouteProfile::Foot => "foot",
        };

        let mut coordinates = vec![format!("{},{}", start.longitude, start.latitude)];
        for wp in &waypoints {
            coordinates.push(format!("{},{}", wp.longitude, wp.latitude));
        }
        coordinates.push(format!("{},{}", end.longitude, end.latitude));
        let coords_str = coordinates.join(";");

        let url = format!(
            "https://router.project-osrm.org/route/v1/{}/{}?overview=full&geometries=geojson&steps=true&alternatives={}",
            profile_str, coords_str, alternatives
        );

        assert!(url.contains("driving"));
        assert!(url.contains("13.405,52.52"));
        assert!(url.contains("13.41,52.53"));
        assert!(url.contains("alternatives=false"));
    }

    #[test]
    fn test_build_osrm_url_with_waypoints() {
        let start = GeoCoordinate::new(52.52, 13.405);
        let end = GeoCoordinate::new(52.55, 13.45);
        let waypoints = vec![
            GeoCoordinate::new(52.53, 13.41),
            GeoCoordinate::new(52.54, 13.42),
        ];

        let mut coordinates = vec![format!("{},{}", start.longitude, start.latitude)];
        for wp in &waypoints {
            coordinates.push(format!("{},{}", wp.longitude, wp.latitude));
        }
        coordinates.push(format!("{},{}", end.longitude, end.latitude));
        let coords_str = coordinates.join(";");

        let expected = "13.405,52.52;13.41,52.53;13.42,52.54;13.45,52.55";
        assert_eq!(coords_str, expected);
    }

    #[test]
    fn test_route_result_default() {
        let route = RouteResult::default();

        assert_eq!(route.distance, 0.0);
        assert_eq!(route.duration, 0.0);
        assert!(route.geometry.points.is_empty());
        assert!(route.legs.is_empty());
        assert!(route.weight_name.is_empty());
    }

    #[test]
    fn test_route_step_construction() {
        let step = RouteStep {
            instruction: "Turn right".to_string(),
            distance: 100.0,
            duration: 30.0,
            name: "Main Street".to_string(),
            maneuver_type: "turn".to_string(),
            coordinate: GeoCoordinate::new(52.52, 13.405),
        };

        assert_eq!(step.instruction, "Turn right");
        assert_eq!(step.distance, 100.0);
        assert_eq!(step.duration, 30.0);
        assert_eq!(step.name, "Main Street");
    }

    #[test]
    fn test_route_leg_construction() {
        let steps = vec![
            RouteStep {
                instruction: "Start".to_string(),
                distance: 50.0,
                duration: 15.0,
                name: "Start Street".to_string(),
                maneuver_type: "depart".to_string(),
                coordinate: GeoCoordinate::new(52.52, 13.405),
            },
            RouteStep {
                instruction: "Arrive".to_string(),
                distance: 0.0,
                duration: 0.0,
                name: "End Street".to_string(),
                maneuver_type: "arrive".to_string(),
                coordinate: GeoCoordinate::new(52.53, 13.41),
            },
        ];

        let leg = RouteLeg {
            distance: 500.0,
            duration: 120.0,
            summary: "Main Street, Side Street".to_string(),
            steps,
        };

        assert_eq!(leg.distance, 500.0);
        assert_eq!(leg.duration, 120.0);
        assert_eq!(leg.steps.len(), 2);
    }

    #[test]
    fn test_parse_osrm_geometry_to_coordinates() {
        let geojson_coordinates: Vec<Vec<f64>> =
            vec![vec![13.405, 52.52], vec![13.41, 52.53], vec![13.42, 52.54]];

        let geometry: Vec<GeoCoordinate> = geojson_coordinates
            .iter()
            .map(|c| GeoCoordinate::new(c[1], c[0]))
            .collect();

        assert_eq!(geometry.len(), 3);
        assert_eq!(geometry[0].latitude, 52.52);
        assert_eq!(geometry[0].longitude, 13.405);
        assert_eq!(geometry[2].latitude, 52.54);
        assert_eq!(geometry[2].longitude, 13.42);
    }
}

#[cfg(all(test, feature = "execute"))]
mod geometry_tests {
    use super::super::{
        match_trace::OsrmMatchTraceNode,
        nearest::OsrmNearestNode,
        osrm::{OsrmRoute, build_coordinate_string, map_osrm_routes, set_route_geometries},
        plan_route::PlanRouteNode,
        table::OsrmTableNode,
        trip::OsrmTripNode,
    };
    use crate::geo::pins::{coordinates_input, tests::execution_context};
    use flow_like::flow::{node::NodeLogic, pin::ValueType, variable::VariableType};
    use flow_like_types::{
        Value,
        geometry::{GeometryKind, marker},
        json::json,
    };
    use std::sync::Arc;

    #[test]
    fn routing_spatial_pins_use_geometry_without_legacy_coordinate_pins() {
        let definitions = [
            (
                PlanRouteNode::new().get_node(),
                vec![
                    ("start_geometry", ValueType::Normal, false),
                    ("end_geometry", ValueType::Normal, false),
                    ("waypoint_geometries", ValueType::Array, true),
                ],
                vec!["start", "end", "waypoints", "geometry"],
            ),
            (
                OsrmNearestNode::new().get_node(),
                vec![("geometry", ValueType::Normal, false)],
                vec!["coordinate"],
            ),
            (
                OsrmTableNode::new().get_node(),
                vec![("geometries", ValueType::Array, false)],
                vec!["coordinates"],
            ),
            (
                OsrmTripNode::new().get_node(),
                vec![("geometries", ValueType::Array, false)],
                vec!["coordinates", "geometry"],
            ),
            (
                OsrmMatchTraceNode::new().get_node(),
                vec![("geometries", ValueType::Array, false)],
                vec!["coordinates"],
            ),
        ];
        for (node, inputs, removed_names) in definitions {
            assert_eq!(node.version, Some(2));
            for (index, (name, container, optional)) in inputs.into_iter().enumerate() {
                let geometry = node.get_pin_by_name(name).unwrap();
                assert_eq!(geometry.data_type, VariableType::Geometry);
                assert_eq!(
                    geometry.schema.as_deref(),
                    Some(marker(GeometryKind::Point))
                );
                assert_eq!(geometry.value_type, container);
                assert_eq!(geometry.is_optional(), optional);
                assert_eq!(geometry.index, index as u16 + 2);
                if optional {
                    assert_eq!(geometry.default_value.as_deref(), Some(b"[]".as_slice()));
                } else {
                    assert!(geometry.default_value.is_none());
                }
            }
            for name in removed_names {
                assert!(
                    node.get_pin_by_name(name).is_none(),
                    "{} still exposes {name}",
                    node.name
                );
            }
        }
    }

    #[tokio::test]
    async fn routing_geometry_arrays_require_points_and_preserve_order() {
        let definitions: Vec<Arc<dyn NodeLogic>> = vec![
            Arc::new(OsrmTableNode::new()),
            Arc::new(OsrmTripNode::new()),
            Arc::new(OsrmMatchTraceNode::new()),
        ];
        for logic in definitions {
            let mut context = execution_context(logic).await;
            assert!(coordinates_input(&context, "geometries").await.is_err());
            context
                .set_pin_value(
                    "geometries",
                    json!([
                        {"type":"Point","coordinates":[13.405,52.52]},
                        {"type":"Point","coordinates":[-74.0,40.7]},
                    ]),
                )
                .await
                .unwrap();
            let points = coordinates_input(&context, "geometries").await.unwrap();
            assert_eq!(build_coordinate_string(&points), "13.405,52.52;-74,40.7");
            context
                .set_pin_value("geometries", json!([]))
                .await
                .unwrap();
            assert!(
                coordinates_input(&context, "geometries")
                    .await
                    .unwrap()
                    .is_empty()
            );
            let pin = context.get_pin_by_name("geometries").await.unwrap();
            context.override_pin_value(pin.id(), json!([null]));
            assert!(coordinates_input(&context, "geometries").await.is_err());
        }
    }

    fn service_routes() -> Vec<OsrmRoute> {
        flow_like_types::json::from_value(json!([
            {"distance":100.0,"duration":20.0,"geometry":{"coordinates":[[13.405,52.52],[13.41,52.53]]},"legs":[],"weight_name":"routability"},
            {"distance":150.0,"duration":25.0,"geometry":{"coordinates":[[13.405,52.52],[13.42,52.54],[13.41,52.53]]},"legs":[],"weight_name":"routability"}
        ])).unwrap()
    }

    #[tokio::test]
    async fn service_routes_produce_linestrings_and_empty_results_clear_primary() {
        let definitions: Vec<Arc<dyn NodeLogic>> = vec![
            Arc::new(PlanRouteNode::new()),
            Arc::new(OsrmTripNode::new()),
            Arc::new(OsrmMatchTraceNode::new()),
        ];
        for logic in definitions {
            let mut context = execution_context(logic).await;
            let routes = map_osrm_routes(service_routes());
            set_route_geometries(&mut context, &routes).await.unwrap();
            let primary: Value = context.evaluate_pin("geometry_out").await.unwrap();
            assert_eq!(
                primary,
                json!({"type":"LineString","coordinates":[[13.405,52.52],[13.41,52.53]]})
            );
            let lines: Vec<Value> = context.evaluate_pin("route_geometries").await.unwrap();
            assert_eq!(lines.len(), 2);
            assert_eq!(lines[0], primary);
            assert_eq!(lines[1]["coordinates"][1], json!([13.42, 52.54]));
            set_route_geometries(&mut context, &[]).await.unwrap();
            assert!(context.evaluate_pin::<Value>("geometry_out").await.is_err());
            assert_eq!(
                context
                    .evaluate_pin::<Value>("route_geometries")
                    .await
                    .unwrap(),
                json!([])
            );
        }
    }

    #[test]
    fn malformed_osrm_positions_return_parse_errors() {
        let mut routes = flow_like_types::json::to_value(json!({
            "distance":100.0,"duration":20.0,"geometry":{"coordinates":[[13.405],[13.41,52.53]]},"legs":[],"weight_name":"routability"
        })).unwrap();
        assert!(flow_like_types::json::from_value::<OsrmRoute>(routes.clone()).is_err());
        routes["geometry"]["coordinates"][0] = json!([13.405, 52.52]);
        assert!(flow_like_types::json::from_value::<OsrmRoute>(routes).is_ok());
    }

    #[tokio::test]
    async fn geometry_only_route_inputs_are_evaluated_and_failed_retry_clears_lines() {
        let logic = Arc::new(PlanRouteNode::new());
        let mut context = execution_context(logic.clone()).await;
        context
            .set_pin_value(
                "start_geometry",
                json!({"type":"Point","coordinates":[13.405,52.52]}),
            )
            .await
            .unwrap();
        context
            .set_pin_value(
                "end_geometry",
                json!({"type":"Point","coordinates":[13.41,52.53]}),
            )
            .await
            .unwrap();
        context
            .set_pin_value("profile", json!("invalid"))
            .await
            .unwrap();
        set_route_geometries(&mut context, &map_osrm_routes(service_routes()))
            .await
            .unwrap();
        let error = logic.run(&mut context).await.unwrap_err();
        assert!(error.to_string().contains("Unsupported profile"), "{error}");
        assert!(context.evaluate_pin::<Value>("geometry_out").await.is_err());
        assert!(
            context
                .evaluate_pin::<Value>("route_geometries")
                .await
                .is_err()
        );
    }
}
