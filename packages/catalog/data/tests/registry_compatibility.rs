//! Keep every pre-split public node path and its catalog position available.

use flow_like_catalog_data::{self as catalog, NodeLogic};

macro_rules! assert_legacy_registry {
    (
        public: [$($node:path),* $(,)?],
        private: [$($private_name:literal),* $(,)?] $(,)?
    ) => {
        #[test]
        #[allow(clippy::default_constructed_unit_structs)]
        fn facade_preserves_public_nodes_and_registration_order() {
            let mut expected = vec![$(<$node>::default().get_node().name),*];
            expected.extend([$($private_name.to_owned()),*]);
            for (entry_point, nodes) in [
                ("collect_nodes", catalog::collect_nodes()),
                ("get_catalog", catalog::get_catalog()),
            ] {
                // New nodes may be added between legacy entries. Preserve every old
                // entry and their relative order across the split catalog facade.
                let actual: Vec<_> = nodes.iter().map(|node| node.get_node().name)
                    .filter(|name| expected.contains(name)).collect();
                assert_eq!(actual, expected, "{entry_point} changed the data catalog");
            }
        }
    };
}

include!("fixtures/legacy_node_paths.rs");

#[test]
fn database_reference_nodes_are_registered_once() {
    let nodes = catalog::collect_nodes();
    for name in [
        "database_reference",
        "database_versions",
        "database_branches",
        "database_tags",
        "database_checkout",
        "database_snapshot",
        "database_create_branch",
        "database_delete_branch",
        "database_create_tag",
        "database_update_tag",
        "database_delete_tag",
        "database_restore",
        "database_cleanup_versions",
        "database_compare",
        "database_clone",
    ] {
        let matches: Vec<_> = nodes
            .iter()
            .map(|logic| logic.get_node())
            .filter(|node| node.name == name)
            .collect();
        assert_eq!(matches.len(), 1, "Expected one registration for {name}");
        assert!(matches[0].get_pin_by_name("reference").is_some());
        assert!(matches[0].get_pin_by_name("database").is_some());
    }
}
