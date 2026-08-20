#[cfg(all(test, feature = "execute"))]
mod search_tests {
    use crate::geo::{
        BoundingBox, GeoCoordinate,
        search::reverse_geocode::{Address, ReverseGeocodeResult},
        search::search_location::SearchResult,
    };

    const BERLIN_LAT: f64 = 52.52;
    const BERLIN_LNG: f64 = 13.405;

    #[test]
    fn test_search_result_default() {
        let result = SearchResult::default();

        assert!(result.display_name.is_empty());
        assert_eq!(result.coordinate.latitude, 0.0);
        assert_eq!(result.coordinate.longitude, 0.0);
        assert!(result.place_type.is_empty());
        assert_eq!(result.importance, 0.0);
        assert!(result.bounding_box.is_none());
        assert!(result.osm_id.is_none());
        assert!(result.osm_type.is_none());
    }

    #[test]
    fn test_search_result_construction() {
        let result = SearchResult {
            display_name: "Berlin, Germany".to_string(),
            coordinate: GeoCoordinate::new(BERLIN_LAT, BERLIN_LNG),
            place_type: "city".to_string(),
            importance: 0.85,
            bounding_box: Some(BoundingBox::new(52.3, 13.1, 52.7, 13.8)),
            osm_id: Some(12345678),
            osm_type: Some("relation".to_string()),
        };

        assert_eq!(result.display_name, "Berlin, Germany");
        assert_eq!(result.coordinate.latitude, BERLIN_LAT);
        assert_eq!(result.coordinate.longitude, BERLIN_LNG);
        assert_eq!(result.place_type, "city");
        assert_eq!(result.importance, 0.85);
        assert!(result.bounding_box.is_some());
        assert_eq!(result.osm_id, Some(12345678));
    }

    #[test]
    fn test_reverse_geocode_result_default() {
        let result = ReverseGeocodeResult::default();

        assert!(result.display_name.is_empty());
        assert_eq!(result.coordinate.latitude, 0.0);
        assert_eq!(result.coordinate.longitude, 0.0);
        assert!(result.osm_id.is_none());
        assert!(result.osm_type.is_none());
    }

    #[test]
    fn test_address_default() {
        let address = Address::default();

        assert!(address.house_number.is_none());
        assert!(address.road.is_none());
        assert!(address.suburb.is_none());
        assert!(address.city.is_none());
        assert!(address.county.is_none());
        assert!(address.state.is_none());
        assert!(address.postcode.is_none());
        assert!(address.country.is_none());
        assert!(address.country_code.is_none());
    }

    #[test]
    fn test_address_construction() {
        let address = Address {
            house_number: Some("123".to_string()),
            road: Some("Main Street".to_string()),
            suburb: Some("Mitte".to_string()),
            city: Some("Berlin".to_string()),
            county: None,
            state: Some("Berlin".to_string()),
            postcode: Some("10115".to_string()),
            country: Some("Germany".to_string()),
            country_code: Some("de".to_string()),
        };

        assert_eq!(address.house_number, Some("123".to_string()));
        assert_eq!(address.road, Some("Main Street".to_string()));
        assert_eq!(address.city, Some("Berlin".to_string()));
        assert_eq!(address.country_code, Some("de".to_string()));
    }

    #[test]
    fn test_bounding_box_contains() {
        let bbox = BoundingBox::new(52.0, 13.0, 53.0, 14.0);

        // Point inside
        let inside = GeoCoordinate::new(52.5, 13.5);
        assert!(bbox.contains(&inside));

        // Point outside (north)
        let outside_north = GeoCoordinate::new(54.0, 13.5);
        assert!(!bbox.contains(&outside_north));

        // Point outside (east)
        let outside_east = GeoCoordinate::new(52.5, 15.0);
        assert!(!bbox.contains(&outside_east));

        // Point on boundary
        let on_boundary = GeoCoordinate::new(52.0, 13.5);
        assert!(bbox.contains(&on_boundary));
    }

    #[test]
    fn test_bounding_box_center() {
        let bbox = BoundingBox::new(52.0, 13.0, 53.0, 14.0);
        let center = bbox.center();

        assert_eq!(center.latitude, 52.5);
        assert_eq!(center.longitude, 13.5);
    }

    #[test]
    fn test_build_nominatim_search_url() {
        let query = "Berlin, Germany";
        let limit = 5;
        let country_codes = "de,at,ch";

        let mut url = format!(
            "https://nominatim.openstreetmap.org/search?q={}&format=json&limit={}&addressdetails=1",
            urlencoding::encode(query),
            limit
        );

        if !country_codes.trim().is_empty() {
            url.push_str(&format!("&countrycodes={}", country_codes.trim()));
        }

        assert!(url.contains("Berlin%2C%20Germany"));
        assert!(url.contains("limit=5"));
        assert!(url.contains("countrycodes=de,at,ch"));
        assert!(url.contains("addressdetails=1"));
    }

    #[test]
    fn test_build_nominatim_search_url_empty_country_codes() {
        let query = "New York";
        let limit = 10;
        let country_codes = "";

        let mut url = format!(
            "https://nominatim.openstreetmap.org/search?q={}&format=json&limit={}&addressdetails=1",
            urlencoding::encode(query),
            limit
        );

        if !country_codes.trim().is_empty() {
            url.push_str(&format!("&countrycodes={}", country_codes.trim()));
        }

        assert!(!url.contains("countrycodes"));
    }

    #[test]
    fn test_build_nominatim_reverse_url() {
        let coordinate = GeoCoordinate::new(BERLIN_LAT, BERLIN_LNG);
        let zoom = 18;

        let url = format!(
            "https://nominatim.openstreetmap.org/reverse?lat={}&lon={}&format=json&addressdetails=1&zoom={}",
            coordinate.latitude, coordinate.longitude, zoom
        );

        assert!(url.contains("lat=52.52"));
        assert!(url.contains("lon=13.405"));
        assert!(url.contains("zoom=18"));
    }

    #[test]
    fn test_zoom_level_clamp() {
        let test_cases = vec![
            (-5_i64, 0_i64),
            (0, 0),
            (10, 10),
            (18, 18),
            (25, 18),
            (100, 18),
        ];

        for (input, expected) in test_cases {
            let clamped = input.clamp(0, 18);
            assert_eq!(clamped, expected);
        }
    }

    #[test]
    fn test_limit_clamp() {
        let test_cases = vec![
            (-10_i64, 1_u32),
            (0, 1),
            (1, 1),
            (25, 25),
            (50, 50),
            (100, 50),
        ];

        for (input, expected) in test_cases {
            let clamped = input.clamp(1, 50) as u32;
            assert_eq!(clamped, expected);
        }
    }

    #[test]
    fn test_parse_lat_lon_strings() {
        let lat_str = "52.520008";
        let lon_str = "13.404954";

        let lat: f64 = lat_str.parse().unwrap_or(0.0);
        let lon: f64 = lon_str.parse().unwrap_or(0.0);

        assert!((lat - 52.520008).abs() < 0.000001);
        assert!((lon - 13.404954).abs() < 0.000001);
    }

    #[test]
    fn test_parse_lat_lon_invalid_returns_default() {
        let lat_str = "invalid";
        let lon_str = "not_a_number";

        let lat: f64 = lat_str.parse().unwrap_or(0.0);
        let lon: f64 = lon_str.parse().unwrap_or(0.0);

        assert_eq!(lat, 0.0);
        assert_eq!(lon, 0.0);
    }

    #[test]
    fn test_parse_bounding_box_from_strings() {
        let bb_strings = [
            "52.3".to_string(),
            "52.7".to_string(),
            "13.1".to_string(),
            "13.8".to_string(),
        ];

        if bb_strings.len() == 4 {
            let bbox = BoundingBox::new(
                bb_strings[0].parse().unwrap_or(0.0),
                bb_strings[2].parse().unwrap_or(0.0),
                bb_strings[1].parse().unwrap_or(0.0),
                bb_strings[3].parse().unwrap_or(0.0),
            );

            assert_eq!(bbox.min_lat, 52.3);
            assert_eq!(bbox.max_lat, 52.7);
            assert_eq!(bbox.min_lon, 13.1);
            assert_eq!(bbox.max_lon, 13.8);
        }
    }

    #[test]
    fn test_url_encoding_special_characters() {
        let query_with_spaces = "New York City";
        let encoded = urlencoding::encode(query_with_spaces);
        assert_eq!(encoded, "New%20York%20City");

        let query_with_ampersand = "Street & Avenue";
        let encoded = urlencoding::encode(query_with_ampersand);
        assert_eq!(encoded, "Street%20%26%20Avenue");

        let query_with_unicode = "München, Deutschland";
        let encoded = urlencoding::encode(query_with_unicode);
        assert!(encoded.contains("%C3%BC")); // ü encoded
    }
}
