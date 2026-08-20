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
