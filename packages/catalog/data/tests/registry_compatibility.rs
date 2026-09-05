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
                let actual: Vec<_> = nodes.iter().map(|node| node.get_node().name).collect();
                assert_eq!(actual, expected, "{entry_point} changed the data catalog");
            }
        }
    };
}

include!("fixtures/legacy_node_paths.rs");
