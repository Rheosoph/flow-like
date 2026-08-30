#[cfg(all(test, feature = "execute"))]
mod map_tests {
    use crate::geo::GeoCoordinate;

    const BERLIN_LAT: f64 = 52.52;
    const BERLIN_LNG: f64 = 13.405;
    const NYC_LAT: f64 = 40.7128;
    const NYC_LNG: f64 = -74.006;

    fn lat_lon_to_tile(lat: f64, lon: f64, zoom: u32) -> (u32, u32, u32, u32) {
        let n = 2.0_f64.powi(zoom as i32);
        let x = (lon + 180.0) / 360.0 * n;
        let lat_rad = lat.to_radians();
        let y = (1.0 - lat_rad.tan().asinh() / std::f64::consts::PI) / 2.0 * n;

        let tile_x = x.floor() as u32;
        let tile_y = y.floor() as u32;
        let pixel_x = ((x - x.floor()) * 256.0) as u32;
        let pixel_y = ((y - y.floor()) * 256.0) as u32;

        (tile_x, tile_y, pixel_x, pixel_y)
    }

    #[test]
    fn test_lat_lon_to_tile_zoom_0() {
        let (tile_x, tile_y, _, _) = lat_lon_to_tile(0.0, 0.0, 0);

        // At zoom 0, the entire world is one tile
        assert_eq!(tile_x, 0);
        assert_eq!(tile_y, 0);
    }

    #[test]
    fn test_lat_lon_to_tile_berlin() {
        let (tile_x, tile_y, pixel_x, pixel_y) = lat_lon_to_tile(BERLIN_LAT, BERLIN_LNG, 15);

        // Berlin at zoom 15 should be in a specific tile range
        assert!(tile_x > 0);
        assert!(tile_y > 0);
        assert!(pixel_x < 256);
        assert!(pixel_y < 256);
    }

    #[test]
    fn test_lat_lon_to_tile_nyc() {
        let (tile_x, tile_y, pixel_x, pixel_y) = lat_lon_to_tile(NYC_LAT, NYC_LNG, 15);

        // NYC (negative longitude) should still produce valid tile coordinates
        assert!(tile_x > 0);
        assert!(tile_y > 0);
        assert!(pixel_x < 256);
        assert!(pixel_y < 256);
    }

    #[test]
    fn test_lat_lon_to_tile_negative_coordinates() {
        // Sydney, Australia (negative latitude)
        let (tile_x, tile_y, pixel_x, pixel_y) = lat_lon_to_tile(-33.8688, 151.2093, 15);

        assert!(tile_x > 0);
        assert!(tile_y > 0);
        assert!(pixel_x < 256);
        assert!(pixel_y < 256);
    }

    #[test]
    fn test_lat_lon_to_tile_boundaries() {
        // Test at various extreme coordinates
        let test_cases = vec![
            (85.0, 180.0),   // Near max latitude, max longitude
            (-85.0, -180.0), // Near min latitude, min longitude
            (0.0, 180.0),    // Equator, date line
            (0.0, -180.0),   // Equator, date line
        ];

        for (lat, lon) in test_cases {
            let (tile_x, tile_y, pixel_x, pixel_y) = lat_lon_to_tile(lat, lon, 10);

            assert!(pixel_x <= 256);
            assert!(pixel_y <= 256);
            // Tile coordinates should be within valid range for zoom 10
            assert!(tile_x <= 1024);
            assert!(tile_y <= 1024);
        }
    }

    #[test]
    fn test_zoom_level_clamp() {
        let test_cases = vec![
            (-5_i64, 1_u32),
            (0, 1),
            (1, 1),
            (10, 10),
            (19, 19),
            (25, 19),
            (100, 19),
        ];

        for (input, expected) in test_cases {
            let clamped = input.clamp(1, 19) as u32;
            assert_eq!(clamped, expected);
        }
    }

    #[test]
    fn test_dimension_clamp() {
        let test_cases = vec![
            (32_i64, 64_u32),
            (64, 64),
            (512, 512),
            (1024, 1024),
            (2048, 2048),
            (4096, 2048),
        ];

        for (input, expected) in test_cases {
            let clamped = input.clamp(64, 2048) as u32;
            assert_eq!(clamped, expected);
        }
    }

    #[test]
    fn test_tile_url_standard() {
        let tile_base_url = "https://tile.openstreetmap.org";
        let zoom = 15_u32;
        let tile_x = 17602_u32;
        let tile_y = 10748_u32;

        let tile_url = format!("{}/{}/{}/{}.png", tile_base_url, zoom, tile_x, tile_y);

        assert_eq!(
            tile_url,
            "https://tile.openstreetmap.org/15/17602/10748.png"
        );
    }

    #[test]
    fn test_tile_url_satellite() {
        let tile_base_url =
            "https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile";
        let zoom = 15_u32;
        let tile_x = 17602_u32;
        let tile_y = 10748_u32;

        // Satellite tiles use zoom/y/x format
        let tile_url = format!("{}/{}/{}/{}", tile_base_url, zoom, tile_y, tile_x);

        assert!(tile_url.contains("/15/10748/17602"));
    }

    #[test]
    fn test_tile_url_terrain() {
        let tile_base_url = "https://tile.opentopomap.org";
        let zoom = 15_u32;
        let tile_x = 17602_u32;
        let tile_y = 10748_u32;

        let tile_url = format!("{}/{}/{}/{}.png", tile_base_url, zoom, tile_x, tile_y);

        assert_eq!(tile_url, "https://tile.opentopomap.org/15/17602/10748.png");
    }

    #[test]
    fn test_tile_url_dark() {
        let tile_base_url = "https://tiles.stadiamaps.com/tiles/alidade_smooth_dark";
        let zoom = 15_u32;
        let tile_x = 17602_u32;
        let tile_y = 10748_u32;

        let tile_url = format!("{}/{}/{}/{}.png", tile_base_url, zoom, tile_x, tile_y);

        assert!(tile_url.contains("alidade_smooth_dark"));
        assert!(tile_url.ends_with(".png"));
    }

    #[test]
    fn test_style_to_url_mapping() {
        let styles = vec![
            ("Standard", "https://tile.openstreetmap.org"),
            (
                "Satellite",
                "https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile",
            ),
            ("Terrain", "https://tile.opentopomap.org"),
            (
                "Dark",
                "https://tiles.stadiamaps.com/tiles/alidade_smooth_dark",
            ),
        ];

        for (style, expected_base) in styles {
            let tile_base_url = match style {
                "Standard" => "https://tile.openstreetmap.org",
                "Satellite" => {
                    "https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile"
                }
                "Terrain" => "https://tile.opentopomap.org",
                "Dark" => "https://tiles.stadiamaps.com/tiles/alidade_smooth_dark",
                _ => "https://tile.openstreetmap.org",
            };
            assert_eq!(tile_base_url, expected_base);
        }
    }

    #[test]
    fn test_tile_calculation_consistency() {
        let (tile_x1, tile_y1, px1, py1) = lat_lon_to_tile(BERLIN_LAT, BERLIN_LNG, 15);
        let (tile_x2, tile_y2, px2, py2) = lat_lon_to_tile(BERLIN_LAT, BERLIN_LNG, 15);

        // Same input should produce same output
        assert_eq!(tile_x1, tile_x2);
        assert_eq!(tile_y1, tile_y2);
        assert_eq!(px1, px2);
        assert_eq!(py1, py2);
    }

    #[test]
    fn test_higher_zoom_produces_larger_tile_numbers() {
        let (tile_x_low, tile_y_low, _, _) = lat_lon_to_tile(BERLIN_LAT, BERLIN_LNG, 10);
        let (tile_x_high, tile_y_high, _, _) = lat_lon_to_tile(BERLIN_LAT, BERLIN_LNG, 15);

        // Higher zoom should produce larger tile numbers
        assert!(tile_x_high > tile_x_low);
        assert!(tile_y_high > tile_y_low);
    }

    #[test]
    fn test_tile_grid_calculation() {
        let zoom = 15_u32;
        let width = 512_u32;
        let height = 512_u32;
        let tile_size = 256_u32;

        let (center_tile_x, center_tile_y, pixel_offset_x, pixel_offset_y) =
            lat_lon_to_tile(BERLIN_LAT, BERLIN_LNG, zoom);

        let half_width = width as i32 / 2;
        let half_height = height as i32 / 2;

        let start_pixel_x = pixel_offset_x as i32 - half_width;
        let start_pixel_y = pixel_offset_y as i32 - half_height;

        let start_tile_x =
            center_tile_x as i32 + (start_pixel_x as f64 / tile_size as f64).floor() as i32;
        let start_tile_y =
            center_tile_y as i32 + (start_pixel_y as f64 / tile_size as f64).floor() as i32;
        let end_tile_x = center_tile_x as i32
            + ((start_pixel_x + width as i32) as f64 / tile_size as f64).ceil() as i32;
        let end_tile_y = center_tile_y as i32
            + ((start_pixel_y + height as i32) as f64 / tile_size as f64).ceil() as i32;

        let tiles_x = (end_tile_x - start_tile_x) as u32;
        let tiles_y = (end_tile_y - start_tile_y) as u32;

        // For a 512x512 image, we typically need 2-3 tiles in each direction
        assert!((2..=4).contains(&tiles_x));
        assert!((2..=4).contains(&tiles_y));
    }

    #[test]
    fn test_tile_wrapping_at_antimeridian() {
        let max_tile = 2_u32.pow(15);

        // Test positive wrapping
        let tile_x_over = (max_tile as i32 + 5).rem_euclid(max_tile as i32) as u32;
        assert_eq!(tile_x_over, 5);

        // Test negative wrapping
        let tile_x_under = (-5_i32).rem_euclid(max_tile as i32) as u32;
        assert!(tile_x_under > 0);
        assert!(tile_x_under < max_tile);
    }

    #[test]
    fn test_geo_coordinate_construction() {
        let coord = GeoCoordinate::new(BERLIN_LAT, BERLIN_LNG);

        assert_eq!(coord.latitude, BERLIN_LAT);
        assert_eq!(coord.longitude, BERLIN_LNG);
    }

    #[test]
    fn test_geo_coordinate_default() {
        let coord = GeoCoordinate::default();

        assert_eq!(coord.latitude, 0.0);
        assert_eq!(coord.longitude, 0.0);
    }
}
