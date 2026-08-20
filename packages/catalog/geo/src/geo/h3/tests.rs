#[cfg(all(test, feature = "execute"))]
mod h3_tests {
    use crate::geo::GeoCoordinate;
    use h3o::{CellIndex, LatLng, Resolution};
    use std::str::FromStr;

    const BERLIN_LAT: f64 = 52.52;
    const BERLIN_LNG: f64 = 13.405;
    const NYC_LAT: f64 = 40.7128;
    const NYC_LNG: f64 = -74.006;

    #[test]
    fn test_latlng_to_cell_valid_coordinates() {
        let latlng = LatLng::new(BERLIN_LAT, BERLIN_LNG).unwrap();
        let res = Resolution::try_from(9_u8).unwrap();
        let cell: CellIndex = latlng.to_cell(res);

        assert!(!cell.to_string().is_empty());
        assert_eq!(u8::from(cell.resolution()), 9);
    }

    #[test]
    fn test_latlng_to_cell_different_resolutions() {
        let latlng = LatLng::new(BERLIN_LAT, BERLIN_LNG).unwrap();

        for resolution in 0..=15 {
            let res = Resolution::try_from(resolution).unwrap();
            let cell: CellIndex = latlng.to_cell(res);
            assert_eq!(u8::from(cell.resolution()), resolution);
        }
    }

    #[test]
    fn test_cell_to_latlng_roundtrip() {
        let original_latlng = LatLng::new(BERLIN_LAT, BERLIN_LNG).unwrap();
        let res = Resolution::try_from(15_u8).unwrap();
        let cell: CellIndex = original_latlng.to_cell(res);

        let center: LatLng = LatLng::from(cell);
        let diff_lat = (center.lat() - BERLIN_LAT).abs();
        let diff_lng = (center.lng() - BERLIN_LNG).abs();

        // At resolution 15, we should be very close (within ~1m)
        assert!(diff_lat < 0.0001);
        assert!(diff_lng < 0.0001);
    }

    #[test]
    fn test_cell_from_string_valid() {
        let latlng = LatLng::new(BERLIN_LAT, BERLIN_LNG).unwrap();
        let res = Resolution::try_from(9_u8).unwrap();
        let cell: CellIndex = latlng.to_cell(res);
        let cell_str = cell.to_string();

        let parsed = CellIndex::from_str(&cell_str);
        assert!(parsed.is_ok());
        assert_eq!(parsed.unwrap(), cell);
    }

    #[test]
    fn test_cell_from_string_invalid() {
        let result = CellIndex::from_str("invalid_cell_index");
        assert!(result.is_err());
    }

    #[test]
    fn test_cell_area_calculations() {
        let latlng = LatLng::new(BERLIN_LAT, BERLIN_LNG).unwrap();
        let res = Resolution::try_from(9_u8).unwrap();
        let cell: CellIndex = latlng.to_cell(res);

        let area_m2 = cell.area_m2();
        assert!(area_m2 > 0.0);

        // Resolution 9 cells are approximately 0.1 km² = 100,000 m²
        assert!(area_m2 > 50_000.0 && area_m2 < 200_000.0);
    }

    #[test]
    fn test_cell_area_unit_conversions() {
        let latlng = LatLng::new(BERLIN_LAT, BERLIN_LNG).unwrap();
        let res = Resolution::try_from(9_u8).unwrap();
        let cell: CellIndex = latlng.to_cell(res);

        let area_m2 = cell.area_m2();

        let area_km2 = area_m2 / 1_000_000.0;
        let area_mi2 = area_m2 / 2_589_988.11;
        let area_ha = area_m2 / 10_000.0;
        let area_acres = area_m2 / 4_046.86;

        assert!(area_km2 < area_m2);
        assert!(area_mi2 < area_km2);
        assert!(area_ha < area_m2 && area_ha > area_km2);
        assert!(area_acres > area_ha);
    }

    #[test]
    fn test_cell_boundary_typical_hexagon() {
        let latlng = LatLng::new(BERLIN_LAT, BERLIN_LNG).unwrap();
        let res = Resolution::try_from(9_u8).unwrap();
        let cell: CellIndex = latlng.to_cell(res);

        let boundary = cell.boundary();
        let coords: Vec<GeoCoordinate> = boundary
            .iter()
            .map(|ll| GeoCoordinate::new(ll.lat(), ll.lng()))
            .collect();

        // Most H3 cells are hexagons (6 vertices), some are pentagons (5 vertices)
        assert!(coords.len() == 5 || coords.len() == 6);

        // All coordinates should be valid
        for coord in &coords {
            assert!(coord.latitude >= -90.0 && coord.latitude <= 90.0);
            assert!(coord.longitude >= -180.0 && coord.longitude <= 180.0);
        }
    }

    #[test]
    fn test_cell_to_parent_valid() {
        let latlng = LatLng::new(BERLIN_LAT, BERLIN_LNG).unwrap();
        let res = Resolution::try_from(9_u8).unwrap();
        let cell: CellIndex = latlng.to_cell(res);

        let parent_res = Resolution::try_from(5_u8).unwrap();
        let parent = cell.parent(parent_res);

        assert!(parent.is_some());
        let parent = parent.unwrap();
        assert_eq!(u8::from(parent.resolution()), 5);
    }

    #[test]
    fn test_cell_to_parent_invalid_higher_resolution() {
        let latlng = LatLng::new(BERLIN_LAT, BERLIN_LNG).unwrap();
        let res = Resolution::try_from(5_u8).unwrap();
        let cell: CellIndex = latlng.to_cell(res);

        // Cannot get parent at a higher resolution
        let higher_res = Resolution::try_from(9_u8).unwrap();
        let parent = cell.parent(higher_res);

        assert!(parent.is_none());
    }

    #[test]
    fn test_cell_to_children() {
        let latlng = LatLng::new(BERLIN_LAT, BERLIN_LNG).unwrap();
        let res = Resolution::try_from(5_u8).unwrap();
        let cell: CellIndex = latlng.to_cell(res);

        let child_res = Resolution::try_from(6_u8).unwrap();
        let children: Vec<CellIndex> = cell.children(child_res).collect();

        // Going one resolution level down produces 7 children
        assert_eq!(children.len(), 7);

        for child in &children {
            assert_eq!(u8::from(child.resolution()), 6);
            // Each child's parent should be the original cell
            let child_parent = child.parent(res).unwrap();
            assert_eq!(child_parent, cell);
        }
    }

    #[test]
    fn test_cell_to_children_multiple_levels() {
        let latlng = LatLng::new(BERLIN_LAT, BERLIN_LNG).unwrap();
        let res = Resolution::try_from(5_u8).unwrap();
        let cell: CellIndex = latlng.to_cell(res);

        let child_res = Resolution::try_from(7_u8).unwrap();
        let children: Vec<CellIndex> = cell.children(child_res).collect();

        // 2 levels down = 7 * 7 = 49 children
        assert_eq!(children.len(), 49);
    }

    #[test]
    fn test_grid_disk_radius_zero() {
        let latlng = LatLng::new(BERLIN_LAT, BERLIN_LNG).unwrap();
        let res = Resolution::try_from(9_u8).unwrap();
        let cell: CellIndex = latlng.to_cell(res);

        let disk: Vec<CellIndex> = cell.grid_disk::<Vec<_>>(0);

        assert_eq!(disk.len(), 1);
        assert_eq!(disk[0], cell);
    }

    #[test]
    fn test_grid_disk_radius_one() {
        let latlng = LatLng::new(BERLIN_LAT, BERLIN_LNG).unwrap();
        let res = Resolution::try_from(9_u8).unwrap();
        let cell: CellIndex = latlng.to_cell(res);

        let disk: Vec<CellIndex> = cell.grid_disk::<Vec<_>>(1);

        // k=1 gives origin + 6 neighbors = 7 cells
        assert_eq!(disk.len(), 7);
        assert!(disk.contains(&cell));
    }

    #[test]
    fn test_grid_disk_larger_radius() {
        let latlng = LatLng::new(BERLIN_LAT, BERLIN_LNG).unwrap();
        let res = Resolution::try_from(9_u8).unwrap();
        let cell: CellIndex = latlng.to_cell(res);

        let disk_k2: Vec<CellIndex> = cell.grid_disk::<Vec<_>>(2);
        let disk_k3: Vec<CellIndex> = cell.grid_disk::<Vec<_>>(3);

        // k=2: 1 + 6 + 12 = 19 cells
        // k=3: 1 + 6 + 12 + 18 = 37 cells
        assert_eq!(disk_k2.len(), 19);
        assert_eq!(disk_k3.len(), 37);
    }

    #[test]
    fn test_grid_distance_same_cell() {
        let latlng = LatLng::new(BERLIN_LAT, BERLIN_LNG).unwrap();
        let res = Resolution::try_from(9_u8).unwrap();
        let cell: CellIndex = latlng.to_cell(res);

        let distance = cell.grid_distance(cell);
        assert!(distance.is_ok());
        assert_eq!(distance.unwrap(), 0);
    }

    #[test]
    fn test_grid_distance_neighbors() {
        let latlng = LatLng::new(BERLIN_LAT, BERLIN_LNG).unwrap();
        let res = Resolution::try_from(9_u8).unwrap();
        let cell: CellIndex = latlng.to_cell(res);

        let neighbors: Vec<CellIndex> = cell.grid_disk::<Vec<_>>(1);

        for neighbor in neighbors {
            let distance = cell.grid_distance(neighbor);
            assert!(distance.is_ok());
            assert!(distance.unwrap() <= 1);
        }
    }

    #[test]
    fn test_grid_distance_different_resolutions_fails() {
        let latlng_a = LatLng::new(BERLIN_LAT, BERLIN_LNG).unwrap();
        let latlng_b = LatLng::new(NYC_LAT, NYC_LNG).unwrap();

        let cell_a: CellIndex = latlng_a.to_cell(Resolution::try_from(9_u8).unwrap());
        let cell_b: CellIndex = latlng_b.to_cell(Resolution::try_from(5_u8).unwrap());

        let distance = cell_a.grid_distance(cell_b);
        assert!(distance.is_err());
    }

    #[test]
    fn test_grid_path_same_cell() {
        let latlng = LatLng::new(BERLIN_LAT, BERLIN_LNG).unwrap();
        let res = Resolution::try_from(9_u8).unwrap();
        let cell: CellIndex = latlng.to_cell(res);

        let path_iter = cell.grid_path_cells(cell);
        assert!(path_iter.is_ok());
        let path: Vec<CellIndex> = path_iter.unwrap().map(|r| r.unwrap()).collect();
        assert_eq!(path.len(), 1);
        assert_eq!(path[0], cell);
    }

    #[test]
    fn test_grid_path_adjacent_cells() {
        let latlng = LatLng::new(BERLIN_LAT, BERLIN_LNG).unwrap();
        let res = Resolution::try_from(9_u8).unwrap();
        let cell: CellIndex = latlng.to_cell(res);

        let neighbors: Vec<CellIndex> = cell.grid_disk::<Vec<_>>(1);
        let neighbor = neighbors.iter().find(|&&n| n != cell).unwrap();

        let path: Vec<CellIndex> = cell
            .grid_path_cells(*neighbor)
            .unwrap()
            .map(|r| r.unwrap())
            .collect();

        assert_eq!(path.len(), 2);
        assert_eq!(path[0], cell);
        assert_eq!(path[1], *neighbor);
    }

    #[test]
    fn test_compact_cells_single() {
        let latlng = LatLng::new(BERLIN_LAT, BERLIN_LNG).unwrap();
        let res = Resolution::try_from(9_u8).unwrap();
        let cell: CellIndex = latlng.to_cell(res);

        let mut cells = vec![cell];
        CellIndex::compact(&mut cells).unwrap();

        assert_eq!(cells.len(), 1);
        assert_eq!(cells[0], cell);
    }

    #[test]
    fn test_compact_cells_all_children() {
        let latlng = LatLng::new(BERLIN_LAT, BERLIN_LNG).unwrap();
        let parent_res = Resolution::try_from(5_u8).unwrap();
        let child_res = Resolution::try_from(6_u8).unwrap();
        let parent: CellIndex = latlng.to_cell(parent_res);

        let mut children: Vec<CellIndex> = parent.children(child_res).collect();
        assert_eq!(children.len(), 7);

        CellIndex::compact(&mut children).unwrap();

        // All 7 children should compact back to the parent
        assert_eq!(children.len(), 1);
        assert_eq!(children[0], parent);
    }

    #[test]
    fn test_compact_cells_partial_children() {
        let latlng = LatLng::new(BERLIN_LAT, BERLIN_LNG).unwrap();
        let parent_res = Resolution::try_from(5_u8).unwrap();
        let child_res = Resolution::try_from(6_u8).unwrap();
        let parent: CellIndex = latlng.to_cell(parent_res);

        let mut children: Vec<CellIndex> = parent.children(child_res).collect();
        let original_len = children.len();
        children.pop(); // Remove one child

        CellIndex::compact(&mut children).unwrap();

        // Should not compact - missing one child
        assert_eq!(children.len(), original_len - 1);
    }

    #[test]
    fn test_edge_length_at_resolution() {
        for resolution in 0..=15_u8 {
            let res = Resolution::try_from(resolution).unwrap();
            let edge_length_m = res.edge_length_m();

            assert!(edge_length_m > 0.0);
        }
    }

    #[test]
    fn test_edge_length_decreases_with_resolution() {
        let mut prev_length = f64::MAX;

        for resolution in 0..=15_u8 {
            let res = Resolution::try_from(resolution).unwrap();
            let edge_length_m = res.edge_length_m();

            assert!(edge_length_m < prev_length);
            prev_length = edge_length_m;
        }
    }

    #[test]
    fn test_edge_length_unit_conversions() {
        let res = Resolution::try_from(9_u8).unwrap();
        let edge_length_m = res.edge_length_m();

        let edge_km = edge_length_m / 1_000.0;
        let edge_mi = edge_length_m / 1_609.344;
        let edge_ft = edge_length_m * 3.28084;

        assert!(edge_km < edge_length_m);
        assert!(edge_mi < edge_km);
        assert!(edge_ft > edge_length_m);
    }

    #[test]
    fn test_cells_to_polygon_single() {
        use h3o::geom::SolventBuilder;

        let latlng = LatLng::new(BERLIN_LAT, BERLIN_LNG).unwrap();
        let res = Resolution::try_from(9_u8).unwrap();
        let cell: CellIndex = latlng.to_cell(res);

        let solvent = SolventBuilder::new().build();
        let multi_poly = solvent.dissolve(std::iter::once(cell)).unwrap();

        assert_eq!(multi_poly.0.len(), 1);
        let polygon = &multi_poly.0[0];
        let exterior_len = polygon.exterior().coords().count();
        assert!(exterior_len == 6 || exterior_len == 7); // 6 vertices + closing point, or pentagon
    }

    #[test]
    fn test_cells_to_polygon_disk() {
        use h3o::geom::SolventBuilder;

        let latlng = LatLng::new(BERLIN_LAT, BERLIN_LNG).unwrap();
        let res = Resolution::try_from(9_u8).unwrap();
        let cell: CellIndex = latlng.to_cell(res);

        let disk: Vec<CellIndex> = cell.grid_disk::<Vec<_>>(1);
        let solvent = SolventBuilder::new().build();
        let multi_poly = solvent.dissolve(disk).unwrap();

        // A disk of k=1 should form a single connected polygon
        assert_eq!(multi_poly.0.len(), 1);
    }

    #[test]
    fn test_cells_to_polygon_disconnected() {
        use h3o::geom::SolventBuilder;

        let berlin = LatLng::new(BERLIN_LAT, BERLIN_LNG).unwrap();
        let nyc = LatLng::new(NYC_LAT, NYC_LNG).unwrap();
        let res = Resolution::try_from(5_u8).unwrap();

        let cell_berlin: CellIndex = berlin.to_cell(res);
        let cell_nyc: CellIndex = nyc.to_cell(res);

        let solvent = SolventBuilder::new().build();
        let multi_poly = solvent.dissolve(vec![cell_berlin, cell_nyc]).unwrap();

        // Two cells far apart should create two separate polygons
        assert_eq!(multi_poly.0.len(), 2);
    }

    #[test]
    fn test_cells_to_polygon_empty() {
        use h3o::geom::SolventBuilder;

        let solvent = SolventBuilder::new().build();
        let result = solvent.dissolve(std::iter::empty());

        // Empty input should succeed with empty output
        assert!(result.is_ok());
        assert!(result.unwrap().0.is_empty());
    }
}
